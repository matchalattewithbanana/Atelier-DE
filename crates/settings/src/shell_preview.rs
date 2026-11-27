//! Offline illustration of native shell preferences. No surface, process,
//! theme provider, or settings file is touched by rendering a draft.
use atelier_config::Appearance;
use gtk::cairo::{Context,FontSlant,FontWeight};

type Color=[f64;3];
fn color(value:&str,fallback:Color)->Color{
    let Some(s)=value.strip_prefix('#').filter(|s|s.len()==6&&s.bytes().all(|b|b.is_ascii_hexdigit()))else{return fallback;};
    let Ok(n)=u32::from_str_radix(s,16)else{return fallback;};
    [f64::from((n>>16)&255)/255.0,f64::from((n>>8)&255)/255.0,f64::from(n&255)/255.0]
}
fn rect(cr:&Context,x:f64,y:f64,w:f64,h:f64,r:f64,c:Color,alpha:f64){
    if w<=0.0||h<=0.0{return;}
    let r=r.clamp(0.0,w.min(h)/2.0);
    cr.new_sub_path();
    if r==0.0{cr.rectangle(x,y,w,h);}else{
        let q=std::f64::consts::FRAC_PI_2;
        cr.arc(x+w-r,y+r,r,-q,0.0);cr.arc(x+w-r,y+h-r,r,0.0,q);
        cr.arc(x+r,y+h-r,r,q,2.0*q);cr.arc(x+r,y+r,r,2.0*q,3.0*q);cr.close_path();
    }
    cr.set_source_rgba(c[0],c[1],c[2],alpha);let _=cr.fill();
}
fn text(cr:&Context,x:f64,y:f64,value:&str,size:f64,c:Color){
    cr.select_font_face("Sans",FontSlant::Normal,FontWeight::Normal);
    cr.set_font_size(size);cr.set_source_rgb(c[0],c[1],c[2]);
    cr.move_to(x,y);let _=cr.show_text(value);
}
/// Scale a fixed illustrative scene to the available area. Logical monitor
/// width controls the same density rule used by the real panel; the scene
/// is not a pixel-exact compositor screenshot or wallpaper decoder.
pub(super) fn draw(cr:&Context,width:i32,height:i32,a:&Appearance,monitor:i32,tags:usize){
    if width<2||height<2{return;}
    let scale=(f64::from(width)/640.0).min(f64::from(height)/360.0);
    let _=cr.save();
    cr.translate((f64::from(width)-640.0*scale)/2.0,(f64::from(height)-360.0*scale)/2.0);
    cr.scale(scale,scale);cr.rectangle(0.0,0.0,640.0,360.0);cr.clip();
    let palette=a.shell_palette.as_ref();
    let surface=palette.map_or([0.10,0.13,0.18],|p|color(&p.surface,[0.10,0.13,0.18]));
    let raised=palette.map_or([0.20,0.24,0.32],|p|color(&p.elevated,[0.20,0.24,0.32]));
    let fg=palette.map_or([0.91,0.93,0.98],|p|color(&p.text,[0.91,0.93,0.98]));
    let accent=color(&a.accent,[0.68,0.76,1.0]);
    rect(cr,0.0,0.0,640.0,360.0,14.0,[0.23,0.28,0.40],1.0);
    let compact=a.panel_density.compact(monitor,tags,a.panel_spacing);
    let panel_h=f64::from(a.panel_height)*0.55;
    let r=f64::from(a.button_radius)*0.55;
    let gap=f64::from(a.panel_spacing)*0.35;
    rect(cr,8.0,8.0,624.0,panel_h,8.0,surface,f64::from(a.transparency.panel)/100.0);
    let ph=(panel_h-8.0).max(8.0);
    let launch_w=if compact{24.0}else{58.0};
    rect(cr,12.0,12.0,launch_w,ph,r,accent,1.0);
    // Abstract glyphs are intentional; they never imply installed apps.
    let mut x=12.0+launch_w+gap;
    for i in 0..tags.min(9){
        rect(cr,x,12.0,16.0,ph,r,if i==0{accent}else{raised},1.0);x+=16.0+gap.min(5.0);
    }
    let mut right=628.0;
    for enabled in [true,a.show_terminal,a.show_audio,a.show_notifications,a.show_media,a.show_battery]{
        if !enabled{continue;}
        let w=if compact{16.0}else{30.0};right-=w;
        rect(cr,right,12.0,w,ph,r,raised,1.0);right-=gap;
    }
    let clock_w=if a.show_date&&!compact{72.0}else{30.0};
    if right-clock_w>x{
        rect(cr,right-clock_w,12.0,clock_w,ph,r,raised,1.0);
    }
    let y=(panel_h+26.0).max(80.0);
    let (lx,lw,lh)=(176.0,288.0,210.0);
    rect(cr,lx,y,lw,lh,14.0,surface,1.0);
    text(cr,lx+14.0,y+22.0,"Applications",15.0,fg);
    rect(cr,lx+14.0,y+30.0,lw-28.0,21.0,10.0,raised,1.0);
    text(cr,lx+22.0,y+44.0,"Search apps",10.0,fg);
    let icon=f64::from(a.launcher_icon_size)*0.65;
    if a.launcher_grid_default{
        for i in 0..6{
            let x=lx+14.0+(i%3) as f64*89.0;let yy=y+62.0+(i/3) as f64*68.0;
            rect(cr,x,yy,82.0,61.0,r,raised,1.0);
            rect(cr,x+(82.0-icon)/2.0,yy+4.0,icon,icon,icon*0.22,accent,1.0);
            rect(cr,x+21.0,yy+50.0,40.0,4.0,2.0,fg,0.8);
        }
    }else{
        for i in 0..3{
            let yy=y+62.0+i as f64*45.0;
            rect(cr,lx+14.0,yy,lw-28.0,41.0,r,raised,1.0);
            let size=icon.min(35.0);
            rect(cr,lx+18.0,yy+(41.0-size)/2.0,size,size,size*0.22,accent,1.0);
            rect(cr,lx+24.0+size,yy+12.0,95.0,5.0,2.0,fg,0.9);
            rect(cr,lx+24.0+size,yy+24.0,136.0,4.0,2.0,fg,0.5);
        }
    }
    if a.dock_enabled{
        let icon=f64::from(a.dock_icon_size)*0.55;
        // Only the first six pins fit this illustration; fixed utilities
        // still appear when the configured pin list is empty.
        let count=a.dock_pins.len().min(6)+2;
        let step=icon+8.0;let w=count as f64*step+10.0;let x=(640.0-w)/2.0;
        rect(cr,x,350.0-icon-16.0,w,icon+16.0,r.max(8.0),surface,f64::from(a.transparency.dock)/100.0);
        for i in 0..count{
            rect(cr,x+9.0+i as f64*step,341.0-icon,icon,icon,r.min(icon/2.0),if i==0{accent}else{raised},1.0);
        }
    }
    let _=cr.restore();
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn color_parser_rejects_partial_or_invalid_hex(){
        assert_eq!(color("#ff0000",[0.0;3]),[1.0,0.0,0.0]);
        assert_eq!(color("bad",[0.5;3]),[0.5;3]);
    }
    #[test]
    fn preview_draws_extreme_valid_geometry_without_cairo_failure(){
        let surface=gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32,320,180).unwrap();
        let cr=Context::new(&surface).unwrap();
        let a=Appearance{panel_height:80,panel_spacing:24,dock_enabled:true,dock_icon_size:64,launcher_icon_size:64,launcher_grid_default:true,dock_pins:(0..32).map(|i|format!("app{i}.desktop")).collect(),..Appearance::default()};
        draw(&cr,320,180,&a,640,9);
        assert!(cr.status().is_ok());
    }
}
