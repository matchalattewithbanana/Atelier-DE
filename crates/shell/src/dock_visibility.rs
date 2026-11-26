//! Dock visibility policy; a separate bottom-center strip reveals a hidden dock.
use atelier_config::tasks::TaskSnapshot;
use gtk::{glib,prelude::*};
use gtk4_layer_shell::{Edge,KeyboardMode,Layer,LayerShell};
use std::{cell::{Cell,RefCell},rc::Rc,time::{Duration,Instant}};

pub struct Visibility{
    source:Option<glib::SourceId>,
    reveal:Option<gtk::ApplicationWindow>,
}
impl Visibility{
    pub fn stop(&mut self){
        if let Some(source)=self.source.take(){source.remove();}
        if let Some(window)=self.reveal.take(){window.destroy();}
    }
}
pub fn start(app:&gtk::Application,dock:&gtk::ApplicationWindow,snapshot:Rc<RefCell<Option<TaskSnapshot>>>,auto_hide:bool,hide_fullscreen:bool,open_menus:Rc<Cell<usize>>)->Visibility{
    let hover=Rc::new(Cell::new(false));
    let deadline=Rc::new(Cell::new(Instant::now()));
    let reveal=if auto_hide{
        let window=gtk::ApplicationWindow::builder().application(app).title("Atelier dock reveal").decorated(false).resizable(false).build();
        window.init_layer_shell();window.set_namespace(Some("atelier-dock-reveal"));window.set_layer(Layer::Top);
        window.set_anchor(Edge::Bottom,true);window.set_keyboard_mode(KeyboardMode::None);window.set_exclusive_zone(0);
        let button=gtk::Button::with_label("▴");button.set_tooltip_text(Some("Reveal Atelier dock"));button.set_size_request(80,12);
        window.set_child(Some(&button));
        let open={let (dock,strip,deadline)=(dock.downgrade(),window.downgrade(),deadline.clone());move||{
            if let Some(dock)=dock.upgrade(){deadline.set(Instant::now()+Duration::from_millis(900));dock.set_visible(true);}
            if let Some(strip)=strip.upgrade(){strip.set_visible(false);}
        }};
        let motion=gtk::EventControllerMotion::new();
        {let open=open.clone();motion.connect_enter(move|_,_,_|open());}
        button.add_controller(motion);button.connect_clicked(move|_|open());
        let motion=gtk::EventControllerMotion::new();
        {let hover=hover.clone();motion.connect_enter(move|_,_,_|hover.set(true));}
        {let (hover,deadline)=(hover.clone(),deadline.clone());motion.connect_leave(move|_|{hover.set(false);deadline.set(Instant::now()+Duration::from_millis(450));});}
        dock.add_controller(motion);
        dock.set_visible(false);window.present();Some(window)
    }else{None};
    let dock=dock.downgrade();
    let owned_reveal=reveal.clone();
    let mut suppressed=false;
    let source=glib::timeout_add_local(Duration::from_millis(100),move||{
        let Some(dock)=dock.upgrade()else{return glib::ControlFlow::Continue;};
        let fullscreen=hide_fullscreen&&snapshot.borrow().as_ref().and_then(|s|s.fullscreen_active).unwrap_or(false);
        if fullscreen{
            if !suppressed{hover.set(false);dock.set_visible(false);if let Some(strip)=&reveal{strip.set_visible(false);}suppressed=true;}
        }else if suppressed{
            suppressed=false;
            if let Some(strip)=&reveal{strip.set_visible(true);}else{dock.set_visible(true);}
        }else if let Some(strip)=&reveal{
            if open_menus.get()>0{deadline.set(Instant::now()+Duration::from_millis(450));return glib::ControlFlow::Continue;}
            if dock.is_visible()&&!hover.get()&&Instant::now()>=deadline.get(){dock.set_visible(false);strip.set_visible(true);}
        }
        glib::ControlFlow::Continue
    });
    Visibility{source:Some(source),reveal:owned_reveal}
}
