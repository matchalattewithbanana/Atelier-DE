//! Original Cairo resource meters, informed by Caelestia's expressive
//! shape-and-fill composition. The Studio supplies sample values only.
use atelier_config::access_theme::Palette;
use gtk::cairo::{Context,LineCap,LineJoin};

#[derive(Clone,Copy)]
enum Kind{Cpu,Memory,Storage}
type Point=(f64,f64);
fn toward(a:Point,b:Point,t:f64)->Point{(a.0+(b.0-a.0)*t,a.1+(b.1-a.1)*t)}
fn shape(cr:&Context,kind:Kind){
    // Unit-square silhouettes, rounded using tangent segments and cubics.
    let points:&[Point]=match kind{
        Kind::Cpu=>&[(0.50,0.04),(0.96,0.37),(0.79,0.94),(0.21,0.94),(0.04,0.37)],
        Kind::Memory=>&[(0.20,0.05),(0.95,0.05),(0.80,0.95),(0.05,0.95)],
        Kind::Storage=>&[(0.27,0.04),(0.73,0.04),(0.97,0.39),(0.50,0.96),(0.03,0.39)],
    };
    cr.new_path();
    let n=points.len();
    for i in 0..n{
        let p=points[i];
        let a=toward(p,points[(i+n-1)%n],0.20);
        let b=toward(p,points[(i+1)%n],0.20);
        if i==0{cr.move_to(a.0,a.1);}else{cr.line_to(a.0,a.1);}
        let c=toward(a,p,0.68);let d=toward(b,p,0.68);
        cr.curve_to(c.0,c.1,d.0,d.1,b.0,b.1);
    }
    cr.close_path();
}
fn rgb(s:&str)->[f64;3]{
    let v=u32::from_str_radix(&s[1..],16).expect("validated palette");
    [f64::from((v>>16)&255)/255.0,f64::from((v>>8)&255)/255.0,f64::from(v&255)/255.0]
}
fn mix(a:[f64;3],b:[f64;3],t:f64)->[f64;3]{std::array::from_fn(|i|a[i]+(b[i]-a[i])*t)}
fn contrast(a:[f64;3],b:[f64;3])->f64{
    let l=|c:[f64;3]|{
        let c=c.map(|v|if v<=0.04045{v/12.92}else{((v+0.055)/1.055).powf(2.4)});
        c[0]*0.2126+c[1]*0.7152+c[2]*0.0722
    };
    let (a,b)=(l(a),l(b));(a.max(b)+0.05)/(a.min(b)+0.05)
}
fn readable_tint(base:[f64;3],tint:[f64;3],fg:[f64;3],mut strength:f64)->[f64;3]{
    for _ in 0..16{
        let candidate=mix(base,tint,strength);
        if contrast(candidate,fg)>=4.5{return candidate;}
        strength*=0.5;
    }
    base
}
fn tones(palette:&Palette,kind:Kind)->([f64;3],[f64;3],[f64;3]){
    let bg=rgb(&palette.background);let text=rgb(&palette.text);let accent=rgb(&palette.accent);
    let tint=match kind{
        Kind::Cpu=>accent,
        Kind::Memory=>mix(accent,[0.84,0.64,0.88],0.45),
        Kind::Storage=>mix(accent,[0.48,0.82,0.73],0.45),
    };
    (readable_tint(bg,tint,text,0.23),readable_tint(bg,tint,text,0.46),text)
}
fn source(cr:&Context,c:[f64;3]){cr.set_source_rgb(c[0],c[1],c[2]);}
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
fn meter(cr:&Context,kind:Kind,percent:u32,palette:&Palette,x:f64,y:f64,size:f64){
    let _=cr.save();cr.translate(x,y);cr.scale(size,size);
    let (base,fill,fg)=tones(palette,kind);
    shape(cr,kind);source(cr,base);let _=cr.fill_preserve();
    let _=cr.save();cr.clip();wave(cr,f64::from(percent)/100.0);source(cr,fill);let _=cr.fill();let _=cr.restore();
    source(cr,fg);icon(cr,kind);
    centered(cr,&format!("{percent}%"),0.50,0.64,0.235);
    if matches!(kind,Kind::Cpu){
        source(cr,rgb(&palette.background));cr.arc(0.85,0.15,0.19,0.0,std::f64::consts::TAU);let _=cr.fill();
        source(cr,rgb(&palette.accent));centered(cr,"54°",0.85,0.15,0.17);
    }
    let _=cr.restore();
}
/// Compact three-meter row: keep square shapes even when the card has
/// extra vertical room. The caller owns the outer surface and clipping.
pub(super) fn draw(cr:&Context,area:(f64,f64,f64,f64),palette:&Palette){
    let (x,y,w,h)=area;
    let padding=(w*0.06).min(h*0.12);let gap=padding*0.55;
    let size=((w-2.0*padding-2.0*gap)/3.0).min(h-2.0*padding);
    if size<1.0{return;}
    let left=x+(w-(3.0*size+2.0*gap))/2.0;
    let top=y+(h-size)/2.0;
    let _=cr.save();cr.rectangle(x,y,w,h);cr.clip();
    for (i,(kind,value)) in [(Kind::Cpu,42),(Kind::Memory,58),(Kind::Storage,73)].into_iter().enumerate(){
        meter(cr,kind,value,palette,left+i as f64*(size+gap),top,size);
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
                let (base,fill,fg)=tones(&theme.palette,kind);
                assert!(contrast(base,fg)>=4.5&&contrast(fill,fg)>=4.5);
            }
        }
    }
    #[test]
    fn draws_into_cpu_surface_at_multiple_sizes(){
        let theme=preset("classic").unwrap();
        for (w,h) in [(360,132),(120,70),(40,20)]{
            let surface=gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32,w,h).unwrap();
            let cr=Context::new(&surface).unwrap();
            draw(&cr,(0.0,0.0,f64::from(w),f64::from(h)),&theme.palette);
            assert!(cr.status().is_ok());
        }
    }
}
