//! Original Cairo resource meters, informed by Caelestia's expressive
//! shape-and-fill composition. The Studio supplies sample values only.
use atelier_config::{access_theme::Palette,design::{Roles,Color,AccentRoles,TypeRole,Shape,Outline,SpringValue,Scheme,Speed,MotionKind}};
use gtk::cairo::{Context,LineCap,LineJoin};

#[derive(Clone,Copy)]
enum Kind{Cpu,Memory,Storage}
fn outline(cr:&Context,shape:&Outline){
    cr.new_path();
    for (i,(x,y)) in shape.points().enumerate(){
        if i==0{cr.move_to(x,y);}else{cr.line_to(x,y);}
    }
    cr.close_path();
}
fn shape(cr:&Context,kind:Kind){
    outline(cr,&match kind{Kind::Cpu=>Shape::Pentagon,Kind::Memory=>Shape::Slanted,Kind::Storage=>Shape::Gem}.outline());
}
/// Offline demonstration state; never reads actual hardware.
pub(super) struct Demo{
    load:SpringValue,
}
impl Default for Demo{
    fn default()->Self{Self{load:SpringValue::new(Scheme::Expressive.spring(Speed::Default,MotionKind::Spatial),0.0)}}
}
impl Demo{
    pub(super) fn set_high(&mut self,high:bool){self.load.retarget(if high{1.0}else{0.0});}
    pub(super) fn advance(&mut self,seconds:f64,reduced:bool)->bool{
        self.load.advance(seconds,reduced);
        !self.load.settled()
    }
    fn sample(&self)->([u32;3],u32,f64){
        let p=self.load.value().clamp(0.0,1.0);
        let lerp=|a:f64,b:f64|(a+(b-a)*p).round() as u32;
        let temperature=lerp(54.0,96.0);
        ([lerp(42.0,96.0),lerp(58.0,87.0),lerp(73.0,91.0)],temperature,((f64::from(temperature)-84.0)/12.0).clamp(0.0,1.0))
    }
}
fn roles(palette:&Palette)->Roles{
    Roles::from_palette(&palette.background,&palette.panel,&palette.text,&palette.muted,&palette.accent).expect("validated palette")
}
fn tones(roles:&Roles,kind:Kind)->AccentRoles{
    match kind{Kind::Cpu=>roles.primary,Kind::Memory=>roles.tertiary,Kind::Storage=>roles.secondary}
}
fn source(cr:&Context,c:Color){let [r,g,b]=c.rgb().map(|v|f64::from(v)/255.0);cr.set_source_rgb(r,g,b);}
fn wave(cr:&Context,value:f64){
    let level=1.0-value.clamp(0.0,1.0);
    let amp=0.025_f64.min(level.min(1.0-level));
    cr.new_path();
    cr.move_to(0.0,level);
    for i in 0..4{
        let x=f64::from(i)*0.25;
        cr.curve_to(x+0.04,level-amp,x+0.085,level-amp,x+0.125,level);
        cr.curve_to(x+0.165,level+amp,x+0.21,level+amp,x+0.25,level);
    }
    cr.line_to(1.0,1.0);cr.line_to(0.0,1.0);cr.close_path();
}
fn icon(cr:&Context,kind:Kind){
    cr.set_line_width(0.026);cr.set_line_cap(LineCap::Round);cr.set_line_join(LineJoin::Round);
    match kind{
        Kind::Cpu=>{
            cr.rectangle(0.40,0.24,0.20,0.19);
            for v in [0.43,0.50,0.57]{
                cr.move_to(v,0.20);cr.line_to(v,0.24);
                cr.move_to(v,0.43);cr.line_to(v,0.47);
            }
            for v in [0.28,0.34,0.40]{
                cr.move_to(0.36,v);cr.line_to(0.40,v);
                cr.move_to(0.60,v);cr.line_to(0.64,v);
            }
        }
        Kind::Memory=>{
            cr.rectangle(0.33,0.26,0.34,0.16);
            for x in [0.39,0.50,0.61]{
                cr.move_to(x,0.29);cr.line_to(x,0.37);
            }
            for x in [0.37,0.43,0.49,0.55,0.61]{
                cr.move_to(x,0.42);cr.line_to(x,0.46);
            }
        }
        Kind::Storage=>{
            cr.rectangle(0.34,0.24,0.32,0.20);
            cr.move_to(0.34,0.36);cr.line_to(0.66,0.36);
            cr.move_to(0.40,0.40);cr.line_to(0.43,0.40);
            cr.move_to(0.49,0.40);cr.line_to(0.52,0.40);
        }
    }
    let _=cr.stroke();
}
fn centered(cr:&Context,text:&str,x:f64,y:f64,size:f64){
    cr.set_font_size(size);
    if let Ok(e)=cr.text_extents(text){
        cr.move_to(x-e.width()/2.0-e.x_bearing(),y-e.height()/2.0-e.y_bearing());
        let _=cr.show_text(text);
    }
}
fn meter(cr:&Context,kind:Kind,percent:u32,temperature:u32,warning_shape:f64,roles:&Roles,x:f64,y:f64,size:f64){
    let _=cr.save();cr.translate(x,y);cr.scale(size,size);
    let t=tones(roles,kind);
    shape(cr,kind);source(cr,t.container.fill);let _=cr.fill_preserve();
    let _=cr.save();cr.clip();wave(cr,f64::from(percent)/100.0);source(cr,t.strong_container.fill);let _=cr.fill();let _=cr.restore();
    source(cr,t.container.on);icon(cr,kind);
    centered(cr,&format!("{percent}%"),0.50,0.64,TypeRole::HeadlineLarge.style(true).size/136.0);
    if matches!(kind,Kind::Cpu){
        let pair=if temperature>=90{roles.error}else{roles.primary.primary};
        let badge=Shape::Circle.outline().interpolate(&Shape::SoftBurst.outline(),warning_shape);
        let _=cr.save();cr.translate(0.65,-0.05);cr.scale(0.40,0.40);
        source(cr,pair.fill);outline(cr,&badge);let _=cr.fill();
        let _=cr.restore();
        source(cr,pair.on);centered(cr,&format!("{temperature}°"),0.85,0.15,0.17);
    }
    let _=cr.restore();
}
/// Compact three-meter row: keep square shapes even when the card has
/// extra vertical room. The caller owns the outer surface and clipping.
pub(super) fn draw(cr:&Context,area:(f64,f64,f64,f64),palette:&Palette,demo:&Demo){
    let roles=roles(palette);
    let (values,temperature,warning_shape)=demo.sample();
    let (x,y,w,h)=area;
    let padding=(w*0.06).min(h*0.12);let gap=padding*0.55;
    let size=((w-2.0*padding-2.0*gap)/3.0).min(h-2.0*padding);
    if size<1.0{return;}
    let left=x+(w-(3.0*size+2.0*gap))/2.0;
    let top=y+(h-size)/2.0;
    let _=cr.save();cr.rectangle(x,y,w,h);cr.clip();
    for (i,(kind,value)) in [Kind::Cpu,Kind::Memory,Kind::Storage].into_iter().zip(values).enumerate(){
        meter(cr,kind,value,temperature,warning_shape,&roles,left+i as f64*(size+gap),top,size);
    }
    let _=cr.restore();
}
#[cfg(test)]
mod tests{
    use super::*;
    use atelier_config::access_theme::{preset,PRESETS};
    #[test]
    fn meter_colors_preserve_readability(){
        for (id,_) in PRESETS{
            let theme=preset(id).unwrap();
            for kind in [Kind::Cpu,Kind::Memory,Kind::Storage]{
                let t=tones(&roles(&theme.palette),kind);
                assert!(t.container.fill.contrast(t.container.on)>=4.5&&t.strong_container.fill.contrast(t.container.on)>=4.5);
            }
        }
    }
    #[test]
    fn sample_retargets_and_reduced_motion_snaps(){
        let mut demo=Demo::default();assert_eq!(demo.sample().0,[42,58,73]);
        demo.set_high(true);assert!(!demo.advance(0.0,true));
        assert_eq!(demo.sample(),([96,87,91],96,1.0));
        demo.set_high(false);assert!(demo.advance(0.016,false));
        assert!(!demo.advance(0.0,true));assert_eq!(demo.sample().1,54);
    }
    #[test]
    fn draws_into_cpu_surface_at_multiple_sizes(){
        let theme=preset("classic").unwrap();
        for (w,h) in [(360,132),(120,70),(40,20)]{
            let surface=gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32,w,h).unwrap();
            let cr=Context::new(&surface).unwrap();
            draw(&cr,(0.0,0.0,f64::from(w),f64::from(h)),&theme.palette,&Demo::default());
            assert!(cr.status().is_ok());
        }
    }
}
