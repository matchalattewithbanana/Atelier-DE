//! Frame-clock icon hover motion. The maximum icon footprint is reserved
//! once; changing the image size never changes the button's hit target.
use atelier_config::design::{MotionKind,Scheme,Speed,SpringValue};
use gtk::{glib,prelude::*};
use std::{cell::{Cell,RefCell},rc::Rc};

#[derive(Clone)]
pub struct Preference(Rc<Cell<bool>>);
impl Preference{
    pub fn new(reduced:bool)->Self{Self(Rc::new(Cell::new(reduced)))}
    pub fn set(&self,reduced:bool){self.0.set(reduced);}
    fn reduced(&self)->bool{
        self.0.get()||gtk::Settings::default().is_some_and(|s|!s.property::<bool>("gtk-enable-animations"))
    }
}
#[derive(Debug,Clone,Copy)]
struct Geometry{base:i32,delta:f64,extent:i32}
impl Geometry{
    fn new(size:u32)->Self{
        let base=size.clamp(16,64) as i32;
        let delta=(f64::from(base)*0.18).clamp(3.0,8.0);
        let extent=base+(delta*1.2).ceil() as i32;
        Self{base,delta,extent}
    }
    fn pixels(self,value:f64)->i32{
        let value=if value.is_finite(){value.clamp(0.0,1.2)}else{0.0};
        (f64::from(self.base)+self.delta*value).round() as i32
    }
}
struct State{spring:SpringValue,hovered:bool,focused:bool,running:bool}
impl State{
    fn new()->Self{Self{spring:SpringValue::new(Scheme::Expressive.spring(Speed::Fast,MotionKind::Spatial),0.0),hovered:false,focused:false,running:false}}
    fn target(&mut self){self.spring.retarget(if self.hovered||self.focused{1.0}else{0.0});}
    fn reset(&mut self){self.hovered=false;self.focused=false;self.target();self.spring.advance(0.0,true);}
}
fn render(image:&gtk::Image,geometry:Geometry,value:f64){
    let size=geometry.pixels(value);
    if image.pixel_size()!=size{image.set_pixel_size(size);}
}
fn retarget(button:&gtk::Button,image:&glib::WeakRef<gtk::Image>,state:&Rc<RefCell<State>>,pref:&Preference,geometry:Geometry){
    let Some(icon)=image.upgrade()else{return;};
    let mut s=state.borrow_mut();s.target();
    if pref.reduced(){
        let value=s.spring.advance(0.0,true);render(&icon,geometry,value);return;
    }
    if s.running||s.spring.settled(){return;}
    s.running=true;drop(s);
    let (image,state,pref)=(image.clone(),state.clone(),pref.clone());
    let previous=Cell::new(None::<i64>);
    button.add_tick_callback(move|_,clock|{
        let mut s=state.borrow_mut();
        let Some(icon)=image.upgrade()else{s.running=false;return glib::ControlFlow::Break;};
        let now=clock.frame_time();
        let dt=previous.replace(Some(now)).map_or(0.0,|old|now.saturating_sub(old) as f64/1_000_000.0);
        let value=s.spring.advance(dt,pref.reduced());
        render(&icon,geometry,value);
        if s.spring.settled(){s.running=false;glib::ControlFlow::Break}else{glib::ControlFlow::Continue}
    });
}
/// Install on the existing primary button, preserving its input and focus
/// behavior. The returned fixed-size slot owns the original GTK image.
pub fn icon_slot(button:&gtk::Button,image:&gtk::Image,size:u32,pref:Preference)->gtk::Box{
    let geometry=Geometry::new(size);
    let slot=gtk::Box::new(gtk::Orientation::Horizontal,0);
    slot.set_size_request(geometry.extent,geometry.extent);
    slot.set_halign(gtk::Align::Center);slot.set_valign(gtk::Align::Center);
    image.set_halign(gtk::Align::Center);image.set_valign(gtk::Align::Center);image.set_hexpand(true);
    render(image,geometry,0.0);slot.append(image);
    let state=Rc::new(RefCell::new(State::new()));
    let weak_image=image.downgrade();
    let motion=gtk::EventControllerMotion::new();
    {
        let (button,image,state,pref)=(button.downgrade(),weak_image.clone(),state.clone(),pref.clone());
        motion.connect_enter(move|_,_,_|{
            if let Some(button)=button.upgrade(){state.borrow_mut().hovered=true;retarget(&button,&image,&state,&pref,geometry);}
        });
    }
    {
        let (button,image,state,pref)=(button.downgrade(),weak_image.clone(),state.clone(),pref.clone());
        motion.connect_leave(move|_|{
            if let Some(button)=button.upgrade(){state.borrow_mut().hovered=false;retarget(&button,&image,&state,&pref,geometry);}
        });
    }
    button.add_controller(motion);
    {
        let (image,state,pref)=(weak_image.clone(),state.clone(),pref.clone());
        button.connect_has_focus_notify(move|button|{
            state.borrow_mut().focused=button.has_focus();
            retarget(button,&image,&state,&pref,geometry);
        });
    }
    button.connect_unmap(move|_|{
        state.borrow_mut().reset();
        if let Some(icon)=weak_image.upgrade(){render(&icon,geometry,0.0);}
    });
    slot
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn hover_geometry_stays_in_reserved_extent(){
        for size in 16..=64{
            let g=Geometry::new(size);
            assert_eq!(g.pixels(0.0),size as i32);
            assert!(g.pixels(1.0)>g.base);
            for value in [-10.0,0.0,0.5,1.0,1.2,100.0,f64::NAN]{
                assert!((g.base..=g.extent).contains(&g.pixels(value)));
            }
        }
    }
    #[test]
    fn interrupted_hover_returns_to_rest_and_focus_preserves_target(){
        let mut s=State::new();s.hovered=true;s.target();s.spring.advance(0.025,false);
        let before=s.spring.value();s.hovered=false;s.target();assert_eq!(s.spring.value(),before);
        s.focused=true;s.target();assert_eq!(s.spring.advance(0.0,true),1.0);
        s.reset();assert_eq!(s.spring.value(),0.0);assert!(s.spring.settled());
    }
}
