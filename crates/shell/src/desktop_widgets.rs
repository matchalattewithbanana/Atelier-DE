//! Opt-in widget group on the native bottom layer, above wallpaper and below applications.
use atelier_config::{DesktopWidgets,WidgetPosition};
use gtk::{glib,prelude::*};
use gtk4_layer_shell::{Edge,KeyboardMode,Layer,LayerShell};
use std::{sync::mpsc,time::Duration};

fn label(text:&str,class:&str)->gtk::Label{
    let label=gtk::Label::new(Some(text));label.set_xalign(0.0);label.set_wrap(true);label.add_css_class(class);label
}
fn card(parent:&gtk::Box,title:&str)->gtk::Box{
    let card=gtk::Box::new(gtk::Orientation::Vertical,6);card.add_css_class("atelier-widget");
    card.append(&label(title,"dim-label"));parent.append(&card);card
}
fn clock(card:&gtk::Box){
    let time=label("","title-1");let date=label("","body");card.append(&time);card.append(&date);
    let (time,date)=(time.downgrade(),date.downgrade());
    let tick=move||{
        let (Some(time),Some(date))=(time.upgrade(),date.upgrade())else{return glib::ControlFlow::Break;};
        if let Ok(now)=glib::DateTime::now_local(){
            if let Ok(t)=now.format("%H:%M"){time.set_text(&t);}
            if let Ok(t)=now.format("%A, %d %B"){date.set_text(&t);}
        }
        glib::ControlFlow::Continue
    };
    tick();glib::timeout_add_local(Duration::from_secs(1),tick);
}
/// A single worker per data source; cancellation is observed between refreshes.
/// GTK receives only text, never service/process handles.
fn refresh(label:&gtk::Label,interval:Duration,read:impl Fn()->String+Send+'static){
    let (cancel,stop)=mpsc::channel::<()>();
    let (tx,rx)=mpsc::sync_channel::<String>(1);
    std::thread::spawn(move||loop{
        let text=read();
        match tx.try_send(text){Ok(())|Err(mpsc::TrySendError::Full(_))=>{},Err(mpsc::TrySendError::Disconnected(_))=>break}
        match stop.recv_timeout(interval){Err(mpsc::RecvTimeoutError::Timeout)=>{},_=>break}
    });
    let weak=label.downgrade();
    glib::timeout_add_local(Duration::from_millis(250),move||{
        let Some(label)=weak.upgrade()else{return glib::ControlFlow::Break;};
        let _=&cancel;
        match rx.try_recv(){
            Ok(text)=>label.set_text(&text),
            Err(mpsc::TryRecvError::Empty)=>{},
            Err(mpsc::TryRecvError::Disconnected)=>{label.set_text("Service unavailable");return glib::ControlFlow::Break;}
        }
        glib::ControlFlow::Continue
    });
}
fn media_summary()->String{
    match atelier_services::media::media_snapshot(){
        Ok(snapshot)=>{
            let selected=snapshot.players.iter().find(|p|p.status.as_deref()==Some("Playing")).or_else(||snapshot.players.first());
            match selected{
                Some(p)=>{
                    let clean=|text:&str|text.chars().filter(|c|!c.is_control()).take(100).collect::<String>();
                    format!("{}\n{}\n{}",clean(p.title.as_deref().unwrap_or("No track information")),clean(p.artist.as_deref().unwrap_or(&p.id)),p.status.as_deref().unwrap_or("Unknown"))
                }
                None=>"No media players".into()
            }
        }
        Err(_)=>"Media service unavailable".into(),
    }
}
fn system_summary()->String{
    let state=atelier_services::system::system_snapshot();
    let memory=match (state.memory_used_kib,state.memory_total_kib){
        (Some(used),Some(total))=>format!("Memory {:.1} / {:.1} GiB",used as f64/1048576.0,total as f64/1048576.0),
        _=>"Memory unavailable".into()
    };
    let load=state.load_1m.map(|v|format!("Load average (1 min): {v:.2}")).unwrap_or_else(||"Load unavailable".into());
    let battery=match atelier_services::system_power_snapshot(){
        Ok(power) if !power.batteries.is_empty()=>format!("Battery {}",power.batteries.iter().map(|b|b.percent.map(|v|format!("{v}%")).unwrap_or("?".into())).collect::<Vec<_>>().join(" / ")),
        Ok(power)=>match power.external_power{Some(true)=>"External power".into(),Some(false)=>"External power disconnected".into(),None=>"No battery reported".into()},
        Err(_)=>"Power unavailable".into()
    };
    format!("{memory}\n{load}\n{battery}")
}
pub fn build(app:&gtk::Application,prefs:&DesktopWidgets)->Option<gtk::ApplicationWindow>{
    if !prefs.enabled||prefs.validate().is_err(){return None;}
    let window=gtk::ApplicationWindow::builder().application(app).title("Atelier widgets").decorated(false).resizable(false).build();
    window.init_layer_shell();window.set_namespace(Some("atelier-desktop-widgets"));window.set_layer(Layer::Bottom);
    window.set_anchor(Edge::Top,true);
    let edge=match prefs.position{WidgetPosition::TopLeft=>Edge::Left,WidgetPosition::TopRight=>Edge::Right};
    window.set_anchor(edge,true);window.set_margin(edge,16);window.set_margin(Edge::Top,16);
    window.set_exclusive_zone(0);window.set_keyboard_mode(KeyboardMode::None);
    let group=gtk::Box::new(gtk::Orientation::Vertical,8);
    group.set_size_request(prefs.width as i32,-1);
    if prefs.clock{clock(&card(&group,"Clock"));}
    if prefs.calendar{
        let card=card(&group,"Calendar");let calendar=gtk::Calendar::new();card.append(&calendar);
    }
    if prefs.media{
        let card=card(&group,"Now playing");let summary=label("Loading media…","body");card.append(&summary);
        let button=gtk::Button::with_label("Media controls");let app=app.downgrade();
        button.connect_clicked(move|_|{if let Some(app)=app.upgrade(){crate::media_controls::show(&app);}});
        card.append(&button);refresh(&summary,Duration::from_secs(15),media_summary);
    }
    if prefs.system{
        let card=card(&group,"System");let summary=label("Loading system…","body");card.append(&summary);
        refresh(&summary,Duration::from_secs(5),system_summary);
    }
    let scroll=gtk::ScrolledWindow::builder().child(&group).hscrollbar_policy(gtk::PolicyType::Never).vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true).max_content_height(440).build();
    window.set_child(Some(&scroll));
    window.present();Some(window)
}
