//! Offline illustration of native shell preferences. No surface, process,
//! theme provider, or settings file is touched by rendering a draft.
use atelier_config::{Appearance,NotificationPosition,NotificationStyle,WidgetPosition};
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
#[derive(Clone,Copy)]
struct Colors{surface:Color,raised:Color,fg:Color,accent:Color}
impl Colors{
    fn new(a:&Appearance)->Self{
        let palette=a.shell_palette.as_ref();
        Self{
            surface:palette.map_or([0.10,0.13,0.18],|p|color(&p.surface,[0.10,0.13,0.18])),
            raised:palette.map_or([0.20,0.24,0.32],|p|color(&p.elevated,[0.20,0.24,0.32])),
            fg:palette.map_or([0.91,0.93,0.98],|p|color(&p.text,[0.91,0.93,0.98])),
            accent:color(&a.accent,[0.68,0.76,1.0]),
        }
    }
}
/// All previews use the same bounded illustrative desktop, with no live
/// theme changes or wallpaper/file access. The caller supplies the draft.
fn scene(cr:&Context,width:i32,height:i32,a:&Appearance,render:impl FnOnce(&Context,&Colors)){
    if width<2||height<2{return;}
    let scale=(f64::from(width)/640.0).min(f64::from(height)/360.0);
    let _=cr.save();
    cr.translate((f64::from(width)-640.0*scale)/2.0,(f64::from(height)-360.0*scale)/2.0);
    cr.scale(scale,scale);cr.rectangle(0.0,0.0,640.0,360.0);cr.clip();
    rect(cr,0.0,0.0,640.0,360.0,14.0,[0.23,0.28,0.40],1.0);
    render(cr,&Colors::new(a));
    let _=cr.restore();
}
#[derive(Clone,Copy,Debug,Default)]
pub(super) struct DockSample{
    pub fullscreen:bool,
    pub revealed:bool,
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum DockPresentation{Hidden,RevealStrip,Visible}
fn dock_presentation(a:&Appearance,sample:DockSample)->DockPresentation{
    if !a.dock_enabled || (a.dock_hide_fullscreen && sample.fullscreen){
        DockPresentation::Hidden
    }else if a.dock_auto_hide && !sample.revealed{
        DockPresentation::RevealStrip
    }else{
        DockPresentation::Visible
    }
}
/// Scale a fixed illustrative scene to the available area. Logical monitor
/// width controls the same density rule used by the real panel; the scene
/// is not a pixel-exact compositor screenshot or wallpaper decoder.
pub(super) fn draw(cr:&Context,width:i32,height:i32,a:&Appearance,monitor:i32,tags:usize,dock_sample:DockSample){
    scene(cr,width,height,a,|cr,colors|{
    let Colors{surface,raised,fg,accent}=*colors;
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
    let dock_state=dock_presentation(a,dock_sample);
    if dock_state==DockPresentation::RevealStrip{
        rect(cr,292.0,348.0,56.0,9.0,4.0,surface,1.0);
        text(cr,316.0,355.0,"^",8.0,fg);
    }
    if dock_state==DockPresentation::Visible{
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
    });
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum WidgetCard{Clock,Calendar,Media,System}
impl WidgetCard{
    fn height(self)->f64{match self{Self::Clock=>65.0,Self::Calendar=>119.0,Self::Media=>82.0,Self::System=>72.0}}
}
fn widget_cards(a:&Appearance)->Vec<WidgetCard>{
    let w=&a.widgets;
    if !w.enabled{return vec![];}
    [(w.clock,WidgetCard::Clock),(w.calendar,WidgetCard::Calendar),(w.media,WidgetCard::Media),(w.system,WidgetCard::System)]
        .into_iter().filter_map(|(enabled,card)|enabled.then_some(card)).collect()
}
pub(super) fn widgets(cr:&Context,width:i32,height:i32,a:&Appearance){
    scene(cr,width,height,a,|cr,c|{
        let cards=widget_cards(a);
        if cards.is_empty(){
            text(cr,24.0,40.0,if a.widgets.enabled{"Select at least one widget"}else{"Widgets disabled"},16.0,[0.95,0.96,1.0]);
            return;
        }
        let w=f64::from(a.widgets.width.clamp(220,360))*0.7;
        let x=if a.widgets.position==WidgetPosition::TopRight{624.0-w}else{16.0};
        let top=16.0;
        // Illustrate the native 440px maximum scroll viewport, not the
        // exact GTK Calendar requisition at every font or theme.
        let max_height=308.0;
        let total=cards.iter().map(|card|card.height()).sum::<f64>()+5.0*(cards.len().saturating_sub(1)) as f64;
        let _=cr.save();cr.rectangle(x,top,w,max_height);cr.clip();
        let mut y=top;
        let alpha=f64::from(a.transparency.widgets)/100.0;
        let r=f64::from(a.button_radius)*0.7;
        for card in cards{
            rect(cr,x,y,w,card.height(),r,c.surface,alpha);
            let left=x+10.0;
            match card{
                WidgetCard::Clock=>{
                    text(cr,left,y+15.0,"Clock",10.0,c.fg);
                    text(cr,left,y+39.0,"10:24",23.0,c.fg);
                    text(cr,left,y+54.0,"Friday, 27 November",10.0,c.fg);
                }
                WidgetCard::Calendar=>{
                    text(cr,left,y+15.0,"Calendar",10.0,c.fg);
                    text(cr,left,y+29.0,"November 2026",12.0,c.fg);
                    let cell=(w-20.0)/7.0;
                    for day in 1..=30{
                        let col=(day-1)%7;let row=(day-1)/7;
                        let dx=left+f64::from(col)*cell;let dy=y+39.0+f64::from(row)*14.0;
                        if day==27{
                            rect(cr,dx,dy,cell-2.0,13.0,5.0,c.raised,1.0);
                            rect(cr,dx,dy+11.0,cell-2.0,2.0,1.0,c.accent,1.0);
                        }
                        text(cr,dx+2.0,dy+10.0,&day.to_string(),9.0,c.fg);
                    }
                }
                WidgetCard::Media=>{
                    text(cr,left,y+15.0,"Now playing",10.0,c.fg);
                    text(cr,left,y+33.0,"Sample track",14.0,c.fg);
                    text(cr,left,y+47.0,"Sample artist · Playing",10.0,c.fg);
                    rect(cr,left,y+54.0,w-20.0,20.0,r,c.raised,1.0);
                    text(cr,left+8.0,y+68.0,"Media controls",10.0,c.fg);
                }
                WidgetCard::System=>{
                    text(cr,left,y+15.0,"System",10.0,c.fg);
                    text(cr,left,y+32.0,"Memory 6.2 / 16 GiB",10.0,c.fg);
                    text(cr,left,y+46.0,"Load average: 0.82",10.0,c.fg);
                    text(cr,left,y+60.0,"Battery 76%",10.0,c.fg);
                }
            }
            y+=card.height()+5.0;
        }
        let _=cr.restore();
        if total>max_height{
            let thumb=max_height*max_height/total;
            rect(cr,x+w-4.0,top,3.0,thumb,1.5,c.accent,0.9);
            text(cr,16.0,348.0,"Additional cards scroll",11.0,[0.95,0.96,1.0]);
        }
    });
}
fn notification_geometry(a:&Appearance,island:bool)->(NotificationStyle,f64,f64,usize){
    let n=&a.notifications;
    let style=n.style_for_launch(island);
    let w=f64::from(n.width.clamp(260,480))*0.7;
    let x=match n.position_for_style(style){
        NotificationPosition::TopLeft=>12.0,
        NotificationPosition::TopCenter=>(640.0-w)/2.0,
        _=>628.0-w,
    };
    let count=n.card_limit(island);
    (style,x,w,count)
}
pub(super) fn notifications(cr:&Context,width:i32,height:i32,a:&Appearance,island:bool,unavailable:bool){
    scene(cr,width,height,a,|cr,c|{
        let n=&a.notifications;
        let panel_h=f64::from(a.panel_height)*0.55;
        rect(cr,8.0,8.0,624.0,panel_h,8.0,c.surface,f64::from(a.transparency.panel)/100.0);
        let mut y=panel_h+20.0;
        if unavailable&&n.hide_when_session_unavailable{
            text(cr,16.0,y+16.0,"Popups hidden in this simulated session",14.0,[0.95,0.96,1.0]);return;
        }
        let (style,x,w,count)=notification_geometry(a,island);
        let alpha=f64::from(n.opacity)/100.0;
        let r=f64::from(n.radius)*0.7;
        if style==NotificationStyle::Island{
            let pw=w.min(170.0);
            rect(cr,x+(w-pw)/2.0,y,pw,22.0,16.0,c.surface,alpha);
            text(cr,x+(w-pw)/2.0+12.0,y+15.0,"Notifications",11.0,c.fg);
            y+=27.0;
        }
        let compact=style==NotificationStyle::Compact;
        let h=if compact{48.0}else{66.0};
        for i in 0..count{
            rect(cr,x,y,w,h,r,c.surface,alpha);
            rect(cr,x+9.0,y+9.0,18.0,18.0,5.0,c.accent,1.0);
            text(cr,x+34.0,y+15.0,["System","Messages","Downloads"][i],10.0,c.fg);
            text(cr,x+34.0,y+30.0,["Update complete","New message","Download finished"][i],12.0,c.fg);
            if !compact{ text(cr,x+34.0,y+46.0,"Sample notification",10.0,c.fg); }
            y+=h+6.0;
        }
    });
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
    fn widget_order_and_visibility_match_native_group(){
        let mut a=Appearance::default();assert!(widget_cards(&a).is_empty());
        a.widgets.enabled=true;a.widgets.media=true;a.widgets.system=true;
        assert_eq!(widget_cards(&a),vec![WidgetCard::Clock,WidgetCard::Calendar,WidgetCard::Media,WidgetCard::System]);
        a.widgets.calendar=false;
        assert_eq!(widget_cards(&a),vec![WidgetCard::Clock,WidgetCard::Media,WidgetCard::System]);
    }
    #[test]
    fn notification_preview_resolves_launch_style_and_placement(){
        let mut a=Appearance::default();a.notifications.max_cards=3;
        let (style,x,w,count)=notification_geometry(&a,false);
        assert_eq!(style,NotificationStyle::Floating);assert_eq!(count,1);assert!((x+w-628.0).abs()<0.001);
        let (style,x,w,count)=notification_geometry(&a,true);
        assert_eq!(style,NotificationStyle::Island);assert_eq!(count,3);assert!((x+w/2.0-320.0).abs()<0.001);
        a.notifications.style=NotificationStyle::Compact;a.notifications.position=NotificationPosition::TopLeft;
        assert_eq!(notification_geometry(&a,true).1,12.0);
    }
    #[test]
    fn dock_preview_respects_disabled_autohide_and_fullscreen(){
        let mut a=Appearance{dock_enabled:false,dock_auto_hide:true,dock_hide_fullscreen:true,..Appearance::default()};
        let revealed=DockSample{revealed:true,fullscreen:false};
        let fullscreen=DockSample{revealed:true,fullscreen:true};
        assert_eq!(dock_presentation(&a,revealed),DockPresentation::Hidden);
        a.dock_enabled=true;
        assert_eq!(dock_presentation(&a,DockSample::default()),DockPresentation::RevealStrip);
        assert_eq!(dock_presentation(&a,revealed),DockPresentation::Visible);
        assert_eq!(dock_presentation(&a,fullscreen),DockPresentation::Hidden);
        assert_eq!(dock_presentation(&a,DockSample{revealed:false,fullscreen:true}),DockPresentation::Hidden);
        a.dock_hide_fullscreen=false;
        assert_eq!(dock_presentation(&a,fullscreen),DockPresentation::Visible);
        assert_eq!(dock_presentation(&a,DockSample{revealed:false,fullscreen:true}),DockPresentation::RevealStrip);
        a.dock_auto_hide=false;
        assert_eq!(dock_presentation(&a,DockSample::default()),DockPresentation::Visible);
    }
    #[test]
    fn preview_draws_extreme_valid_geometry_without_cairo_failure(){
        let surface=gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32,320,180).unwrap();
        let cr=Context::new(&surface).unwrap();
        let a=Appearance{panel_height:80,panel_spacing:24,dock_enabled:true,dock_icon_size:64,launcher_icon_size:64,launcher_grid_default:true,dock_pins:(0..32).map(|i|format!("app{i}.desktop")).collect(),..Appearance::default()};
        draw(&cr,320,180,&a,640,9,DockSample{revealed:true,fullscreen:false});
        let mut a=a;a.dock_auto_hide=true;a.dock_hide_fullscreen=true;
        draw(&cr,320,180,&a,640,9,DockSample::default());
        draw(&cr,320,180,&a,640,9,DockSample{revealed:true,fullscreen:true});
        a.widgets.enabled=true;a.widgets.media=true;a.widgets.system=true;a.widgets.width=360;
        widgets(&cr,320,180,&a);a.notifications.width=480;a.notifications.radius=32;
        notifications(&cr,320,180,&a,true,false);a.notifications.hide_when_session_unavailable=true;
        notifications(&cr,320,180,&a,true,true);
        assert!(cr.status().is_ok());
    }
}
