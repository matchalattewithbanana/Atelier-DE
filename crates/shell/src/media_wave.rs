//! Read-only expressive playback indicator. Native GTK Scale remains
//! the accessible seek input, and only the explicit Seek action changes playback.
use crate::media_artwork::Colors;
use gtk::{glib,prelude::*};
use std::{cell::{Cell,RefCell},rc::Rc};
use std::f64::consts::TAU;

#[derive(Clone,Copy,Default)]
struct State{progress:Option<f64>,playing:bool,colors:Option<Colors>,phase:f64}
pub struct Wave{
    pub widget:gtk::DrawingArea,state:Rc<RefCell<State>>,running:Rc<Cell<bool>>,
}
fn animations()->bool{gtk::Settings::default().is_none_or(|s|s.property::<bool>("gtk-enable-animations"))}
fn paint(cr:&gtk::cairo::Context,w:f64,h:f64,state:State,fallback:gtk::gdk::RGBA){
    let Some(progress)=state.progress.filter(|p|p.is_finite())else{return;};
    if w<16.0||h<16.0{return;}
    let (left,right,mid)=(7.0,w-7.0,h/2.0);
    let x=left+(right-left)*progress.clamp(0.0,1.0);
    let source=|color:atelier_config::design::Color|{
        let [r,g,b]=color.rgb().map(|v|f64::from(v)/255.0);cr.set_source_rgb(r,g,b);
    };
    if let Some(colors)=state.colors{source(colors.background);let _=cr.paint();source(colors.foreground);}
    else{cr.set_source_rgba(f64::from(fallback.red()),f64::from(fallback.green()),f64::from(fallback.blue()),0.9);}
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.set_line_width(5.0);
    if right-x>7.0{
        let _=cr.save();
        if let Some(colors)=state.colors{
            let [r,g,b]=colors.foreground.rgb().map(|v|f64::from(v)/255.0);cr.set_source_rgba(r,g,b,0.30);
        }else{cr.set_source_rgba(f64::from(fallback.red()),f64::from(fallback.green()),f64::from(fallback.blue()),0.30);}
        cr.move_to((x+7.0).min(right),mid);cr.line_to(right,mid);let _=cr.stroke();let _=cr.restore();
    }
    if let Some(colors)=state.colors{source(colors.accent);}
    let length=(x-left-7.0).max(0.0);
    if length>0.0{
        let steps=(length.ceil() as usize).clamp(1,1024);
        for i in 0..=steps{
            let t=i as f64/steps as f64;
            let dx=t*length;
            let envelope=(dx/8.0).min((length-dx)/8.0).clamp(0.0,1.0);
            let y=mid+3.5*envelope*(dx/22.0*TAU+state.phase).sin();
            if i==0{cr.move_to(left+dx,y);}else{cr.line_to(left+dx,y);}
        }
        let _=cr.stroke();
    }
    cr.set_line_width(4.0);cr.move_to(x,mid-10.0);cr.line_to(x,mid+10.0);let _=cr.stroke();
}
impl Wave{
    pub fn new()->Self{
        let widget=gtk::DrawingArea::new();widget.set_content_height(32);widget.set_hexpand(true);widget.set_can_target(false);widget.set_visible(false);
        let state=Rc::new(RefCell::new(State::default()));
        let draw=state.clone();
        widget.set_draw_func(move|widget,cr,w,h|paint(cr,f64::from(w),f64::from(h),*draw.borrow(),widget.style_context().color()));
        Self{widget,state,running:Rc::new(Cell::new(false))}
    }
    pub fn update(&self,progress:Option<f64>,playing:bool,colors:Option<Colors>){
        {let mut state=self.state.borrow_mut();state.progress=progress;state.playing=playing;state.colors=colors;if !animations(){state.phase=0.0;}}
        self.widget.set_visible(progress.is_some());self.widget.queue_draw();
        if progress.is_none()||!playing||!animations()||self.running.replace(true){return;}
        let (state,running)=(self.state.clone(),self.running.clone());
        let previous=Cell::new(None::<i64>);
        self.widget.add_tick_callback(move|widget,clock|{
            let mut s=state.borrow_mut();
            if !s.playing||s.progress.is_none()||!animations(){
                if !animations(){s.phase=0.0;}
                running.set(false);widget.queue_draw();return glib::ControlFlow::Break;
            }
            let now=clock.frame_time();
            let elapsed=previous.replace(Some(now)).map_or(0.0,|old|now.saturating_sub(old) as f64/1_000_000.0);
            s.phase=(s.phase+elapsed.min(0.1)*TAU/2.0)%TAU;
            widget.queue_draw();glib::ControlFlow::Continue
        });
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn wave_draws_at_boundaries_and_tiny_sizes(){
        for (w,h) in [(400,32),(50,24),(2,2)]{
            let surface=gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32,w,h).unwrap();
            let cr=gtk::cairo::Context::new(&surface).unwrap();
            for progress in [None,Some(0.0),Some(0.5),Some(1.0),Some(f64::NAN)]{
                paint(&cr,f64::from(w),f64::from(h),State{progress,..State::default()},gtk::gdk::RGBA::new(1.0,1.0,1.0,1.0));
                assert!(cr.status().is_ok());
            }
        }
    }
}
