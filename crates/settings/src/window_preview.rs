//! Offline window-decoration illustration. No compositor or file access.
//! Geometry is a fixed sample grid, not an implementation of native layout.
use atelier_config::Appearance;
use gtk::cairo::{Context,FontSlant,FontWeight};

#[derive(Clone,Copy,Debug,PartialEq)]
struct Rect{x:f64,y:f64,w:f64,h:f64}
impl Rect{
    fn expand(self,n:f64)->Self{Self{x:self.x-n,y:self.y-n,w:self.w+2.0*n,h:self.h+2.0*n}}
}
const SCALE:f64=0.65;
fn tiles(a:&Appearance)->[Rect;2]{
    let outer=f64::from(a.window_spacing.outer_gap.min(64))*SCALE;
    let gap=f64::from(a.window_spacing.inner_gap.min(48))*SCALE;
    let x=24.0+outer;let y=56.0+outer;
    let w=(592.0-outer*2.0-gap)/2.0;
    let h=280.0-outer*2.0;
    [Rect{x,y,w,h},Rect{x:x+w+gap,y,w,h}]
}
fn rounded(cr:&Context,b:Rect,r:f64){
    let r=r.clamp(0.0,b.w.min(b.h)/2.0);
    cr.new_sub_path();
    if r==0.0{cr.rectangle(b.x,b.y,b.w,b.h);return;}
    let q=std::f64::consts::FRAC_PI_2;
    cr.arc(b.x+b.w-r,b.y+r,r,-q,0.0);
    cr.arc(b.x+b.w-r,b.y+b.h-r,r,0.0,q);
    cr.arc(b.x+r,b.y+b.h-r,r,q,2.0*q);
    cr.arc(b.x+r,b.y+r,r,2.0*q,3.0*q);
    cr.close_path();
}
fn fill(cr:&Context,b:Rect,r:f64,c:[f64;3],alpha:f64){
    rounded(cr,b,r);cr.set_source_rgba(c[0],c[1],c[2],alpha);let _=cr.fill();
}
fn label(cr:&Context,x:f64,y:f64,value:&str,size:f64){
    cr.select_font_face("Sans",FontSlant::Normal,FontWeight::Normal);
    cr.set_font_size(size);cr.set_source_rgb(0.92,0.94,0.98);
    cr.move_to(x,y);let _=cr.show_text(value);
}
fn border_color(a:&Appearance,focused:bool)->[f64;3]{
    a.window_border.color(focused).map(|c|[f64::from(c[0]),f64::from(c[1]),f64::from(c[2])])
        .unwrap_or([0.6,0.65,0.76])
}
fn shadow(cr:&Context,b:Rect,r:f64,radius:f64,opacity:f64){
    if radius<=0.0||opacity<=0.0{return;}
    // Disjoint concentric rings approximate an ambient falloff without
    // stacking alpha at the inner edge or running a compositor blur pass.
    for i in 0..8{
        let outer=radius*f64::from(8-i)/8.0;
        let inner=radius*f64::from(7-i)/8.0;
        let _=cr.save();rounded(cr,b.expand(outer),r+outer);
        rounded(cr,b.expand(inner),r+inner);
        cr.set_fill_rule(gtk::cairo::FillRule::EvenOdd);
        cr.set_source_rgba(0.0,0.0,0.0,opacity*f64::from(i+1)/8.0);
        let _=cr.fill();let _=cr.restore();
    }
}
fn window(cr:&Context,a:&Appearance,b:Rect,focused:bool){
    let r=f64::from(a.window_corner_radius.min(24))*SCALE;
    let bw=f64::from(a.window_border.width.min(8))*SCALE;
    // Draw the outline outside the sample client, keeping client size fixed.
    let outer=b.expand(bw);
    let _=cr.save();
    cr.push_group();
    if bw>0.0{fill(cr,outer,r+bw,border_color(a,focused),1.0);}
    let _=cr.save();rounded(cr,b,r);cr.clip();
    fill(cr,b,0.0,[0.12,0.15,0.21],1.0);
    fill(cr,Rect{h:32.0,..b},0.0,[0.20,0.24,0.33],1.0);
    label(cr,b.x+12.0,b.y+21.0,if focused{"Focused window"}else{"Unfocused window"},13.0);
    for i in 0..4{
        fill(cr,Rect{x:b.x+14.0,y:b.y+50.0+f64::from(i)*20.0,w:(b.w-28.0)*[0.83,0.56,0.72,0.44][i as usize],h:6.0},3.0,[0.51,0.58,0.70],0.65);
    }
    let _=cr.restore();
    let _=cr.pop_group_to_source();
    let _=cr.paint_with_alpha(f64::from(a.transparency.window_alpha(focused,false)));
    let _=cr.restore();
}
pub(super) fn draw(cr:&Context,width:i32,height:i32,a:&Appearance,right_focused:bool){
    if width<2||height<2{return;}
    let scale=(f64::from(width)/640.0).min(f64::from(height)/360.0);
    let _=cr.save();
    cr.translate((f64::from(width)-640.0*scale)/2.0,(f64::from(height)-360.0*scale)/2.0);
    cr.scale(scale,scale);cr.rectangle(0.0,0.0,640.0,360.0);cr.clip();
    fill(cr,Rect{x:0.0,y:0.0,w:640.0,h:360.0},14.0,[0.25,0.30,0.42],1.0);
    // Background detail makes loaded window opacity visible in the sample.
    fill(cr,Rect{x:80.0,y:120.0,w:480.0,h:100.0},20.0,[0.36,0.45,0.60],1.0);
    let valid=a.window_border.validate().is_ok();
    label(cr,24.0,28.0,if valid{"Sample grid · Decorations"}else{"Invalid border color · fallback preview"},14.0);
    let boxes=tiles(a);
    let radius=f64::from(a.window_corner_radius.min(24))*SCALE;
    let bw=f64::from(a.window_border.width.min(8))*SCALE;
    let sr=f64::from(a.window_shadow.radius.min(32))*SCALE;
    let opacity=f64::from(a.window_shadow.opacity.min(60))/100.0;
    // Place shadows behind both clients, then paint the focused client last.
    for b in boxes{shadow(cr,b.expand(bw),radius+bw,sr,opacity);}
    let active=usize::from(right_focused);
    window(cr,a,boxes[1-active],false);
    window(cr,a,boxes[active],true);
    let _=cr.restore();
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn sample_geometry_respects_gap_and_remains_positive(){
        let mut a=Appearance::default();
        let initial=tiles(&a);
        assert_eq!(initial[0].x+initial[0].w,initial[1].x);
        a.window_spacing.inner_gap=48;a.window_spacing.outer_gap=64;
        let [left,right]=tiles(&a);
        assert!((right.x-left.x-left.w-48.0*SCALE).abs()<1e-8);
        assert!(left.x>initial[0].x&&right.x+right.w<initial[1].x+initial[1].w);
        assert!(left.w>0.0&&left.h>0.0);
    }
    #[test]
    fn uses_validated_focus_colors_and_safe_fallback(){
        let mut a=Appearance::default();
        a.window_border.focused="#ff0000".into();a.window_border.unfocused="#00ff00".into();
        assert_eq!(border_color(&a,true),[1.0,0.0,0.0]);
        assert_eq!(border_color(&a,false),[0.0,1.0,0.0]);
        a.window_border.focused="bad".into();
        assert_eq!(border_color(&a,true),[0.6,0.65,0.76]);
    }
    #[test]
    fn draws_both_focus_states_and_invalid_draft(){
        let surface=gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32,320,180).unwrap();
        let cr=Context::new(&surface).unwrap();
        let mut a=Appearance::default();
        draw(&cr,320,180,&a,false);
        a.window_spacing.inner_gap=48;a.window_spacing.outer_gap=64;
        a.window_border.width=8;a.window_corner_radius=24;
        a.window_shadow.radius=32;a.window_shadow.opacity=60;
        a.transparency.focused=70;a.transparency.unfocused=80;
        draw(&cr,320,180,&a,true);
        a.window_border.focused="bad".into();
        draw(&cr,320,180,&a,false);
        assert!(cr.status().is_ok());
    }
}
