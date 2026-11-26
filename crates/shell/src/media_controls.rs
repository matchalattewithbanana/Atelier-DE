//! Capability-aware interactive media window. A single worker owns the
//! session bus, and generation-tagged replies cannot overwrite newer choices.
use crate::mpris::{Action,Bus,Details,Identity,Track,Repeat};
use gtk::{glib,prelude::*};
use crate::{media_artwork::{Artwork,Cache,Colors},media_wave::{Wave,Drag}};
use std::sync::Arc;
use std::{cell::{Cell,RefCell},rc::Rc,sync::mpsc,time::{Duration,Instant}};

enum Request{Select(String),Act(Identity,Action),Artwork(bool)}
struct Reply{
    revision:u64,names:Vec<String>,selected:Option<String>,
    details:Option<Details>,error:Option<String>,art:Option<Arc<Artwork>>,
}
type Command=(u64,Request);
/// Requests wake the private worker context, including disconnect on close.
struct RequestPort{sender:mpsc::SyncSender<Command>,context:glib::MainContext}
impl RequestPort{
    fn send(&self,request:Command)->bool{
        if self.sender.try_send(request).is_err(){return false;}
        self.context.wakeup();true
    }
}
impl Drop for RequestPort{
    fn drop(&mut self){self.context.wakeup();}
}
enum Wake{Request(Command),Refresh,Closed}
/// Keep the main context blocked when idle. The deadline is a compatibility
/// fallback for players that omit signals, and samples Position while playing.
fn wait(context:&glib::MainContext,rx:&mpsc::Receiver<Command>,bus:Option<&Bus>,interval:Duration)->Wake{
    let started=Instant::now();
    let timer=glib::timeout_source_new(interval,Some("atelier-media-refresh"),glib::Priority::DEFAULT,||glib::ControlFlow::Break);
    timer.attach(Some(context));
    // Coalesce a signal burst into at most one refresh per 100ms. A single
    // extra one-shot wakes the context at the end of the coalescing interval.
    let mut debounce=None;
    let result=loop{
        match rx.try_recv(){
            Ok(request)=>break Wake::Request(request),
            Err(mpsc::TryRecvError::Disconnected)=>break Wake::Closed,
            Err(mpsc::TryRecvError::Empty)=>{}
        }
        let elapsed=started.elapsed();
        if elapsed>=interval{break Wake::Refresh;}
        if bus.is_some_and(Bus::has_change){
            if elapsed>=Duration::from_millis(100){break Wake::Refresh;}
            if debounce.is_none(){
                let source=glib::timeout_source_new(Duration::from_millis(100).saturating_sub(elapsed).max(Duration::from_millis(1)),Some("atelier-media-coalesce"),glib::Priority::DEFAULT,||glib::ControlFlow::Break);
                source.attach(Some(context));debounce=Some(source);
            }
        }
        context.iteration(true);
    };
    timer.destroy();if let Some(source)=debounce{source.destroy();}
    result
}
fn worker(context:&glib::MainContext,rx:mpsc::Receiver<Command>,tx:mpsc::SyncSender<Reply>){
    let mut bus=None;
    let mut revision=0;
    let mut selected:Option<String>=None;
    let mut action_error=None;
    let mut artwork=Cache::default();let mut artwork_enabled=false;
    loop{
        let result=(||->anyhow::Result<_>{
            if bus.is_none(){bus=Some(Bus::connect()?);}
            let bus=bus.as_ref().expect("connected");
            bus.take_change();
            let names=bus.players()?;
            if selected.as_ref().is_none_or(|name|!names.contains(name)){selected=names.first().cloned();}
            if selected.is_none(){bus.clear_selection();}
            let details=selected.as_ref().map(|name|bus.selected(name));
            Ok((names,details))
        })();
        let reply=match result{
            Ok((names,details))=>{
                let (details,error)=match details{
                    Some(Ok(d))=>(Some(d),None),
                    Some(Err(e))=>(None,Some(e.to_string())),
                    None=>(None,None),
                };
                let art=artwork.update(artwork_enabled,details.as_ref());
                Reply{revision,names,selected:selected.clone(),details,error:action_error.take().or(error),art}
            }
            Err(e)=>{bus=None;artwork.update(false,None);Reply{revision,names:vec![],selected:None,details:None,error:action_error.take().or(Some(e.to_string())),art:None}}
        };
        let interval=if reply.error.is_some()||reply.details.as_ref().is_some_and(|d|d.status=="Playing"){Duration::from_secs(2)}else{Duration::from_secs(15)};
        if tx.send(reply).is_err(){break;}
        match wait(context,&rx,bus.as_ref(),interval){
            Wake::Request((next,request))=>{
                revision=next;
                match request{
                    Request::Select(name)=>selected=Some(name),
                    Request::Artwork(enabled)=>artwork_enabled=enabled,
                    Request::Act(identity,action)=>{
                        action_error=match &bus{
                            Some(bus)=>bus.action(&identity,action).err().map(|e|e.to_string()),
                            None=>Some("Session bus unavailable".into()),
                        };
                    }
                }
            }
            Wake::Refresh=>{}
            Wake::Closed=>break,
        }
    }
}
fn time(us:i64)->String{
    let seconds=us.max(0)/1_000_000;
    if seconds>=3600{format!("{}:{:02}:{:02}",seconds/3600,(seconds/60)%60,seconds%60)}
    else{format!("{}:{:02}",seconds/60,seconds%60)}
}
fn label(text:&str,class:&str)->gtk::Label{
    let l=gtk::Label::new(Some(text));l.set_xalign(0.0);l.set_wrap(true);l.add_css_class(class);l
}
struct Ui{
    choice:gtk::ComboBoxText,track:gtk::Label,artist:gtk::Label,album:gtk::Label,playback:gtk::Label,
    scale:gtk::Scale,position:gtk::Label,seek:gtk::Button,previous:gtk::Button,toggle:gtk::Button,next:gtk::Button,shuffle:gtk::Button,repeat:gtk::Button,
    artwork_enabled:gtk::CheckButton,cover:gtk::Picture,wave:Wave,art:RefCell<Option<Arc<Artwork>>>,
    status:gtk::Label,names:RefCell<Vec<String>>,details:RefCell<Option<Details>>,
    revision:Cell<u64>,busy:Cell<bool>,updating:Cell<bool>,draft:Cell<bool>,drag_track:RefCell<Option<Track>>,
    requests:RequestPort,
}
impl Ui{
    fn request(&self,request:Request){
        if self.busy.get(){return;}
        let Some(revision)=self.revision.get().checked_add(1)else{return;};
        let switching=matches!(&request,Request::Select(_));
        if self.requests.send((revision,request)){
            if switching{self.set_art(None);}
            self.revision.set(revision);self.busy.set(true);self.draft.set(false);self.drag_track.borrow_mut().take();self.wave.set_interactive(false);
            self.choice.set_sensitive(false);self.artwork_enabled.set_sensitive(false);
            self.wave.update(None,false,None);
            self.disable_actions();self.status.set_text("Updating player…");
        }else{self.status.set_text("Media worker busy or unavailable.");}
    }
    fn disable_actions(&self){
        for button in [&self.previous,&self.toggle,&self.next,&self.seek,&self.shuffle,&self.repeat]{button.set_sensitive(false);}
        self.scale.set_sensitive(false);
    }
    fn apply(&self,reply:Reply){
        if reply.revision!=self.revision.get(){return;}
        self.updating.set(true);self.busy.set(false);
        let keep_draft=self.draft.get()&&{
            let old=self.details.borrow();
            old.as_ref().and_then(|d|d.track.as_ref()).zip(reply.details.as_ref().and_then(|d|d.track.as_ref()))
                .is_some_and(|(a,b)|a==b)&&reply.details.as_ref().is_some_and(|d|d.capabilities.seek)
        };
        self.draft.set(keep_draft);
        if !keep_draft{self.drag_track.borrow_mut().take();}
        if *self.names.borrow()!=reply.names{
            self.choice.remove_all();
            for name in &reply.names{self.choice.append(Some(name),name.strip_prefix("org.mpris.MediaPlayer2.").unwrap_or(name));}
            *self.names.borrow_mut()=reply.names;
        }
        if let Some(id)=reply.selected{self.choice.set_active_id(Some(&id));}else{self.choice.set_active(None);}
        self.choice.set_sensitive(!self.names.borrow().is_empty());self.artwork_enabled.set_sensitive(true);
        self.set_art(if self.artwork_enabled.is_active(){reply.art}else{None});
        *self.details.borrow_mut()=reply.details;
        self.render();
        self.updating.set(false);
        if let Some(error)=reply.error{self.status.set_text(&format!("Media: {error}"));}
        else if self.names.borrow().is_empty(){self.status.set_text("No MPRIS players detected.");}
        else{self.status.set_text("Position and controls follow the selected player.");}
    }
    fn set_art(&self,art:Option<Arc<Artwork>>){
        let mut old=self.art.borrow_mut();
        if old.as_ref().zip(art.as_ref()).is_some_and(|(a,b)|Arc::ptr_eq(a,b)){return;}
        if let Some(image)=art.as_ref(){
            let r=&image.raster;
            let texture=gtk::gdk::MemoryTexture::new(r.width as i32,r.height as i32,gtk::gdk::MemoryFormat::R8g8b8a8,&glib::Bytes::from_owned(r.rgba.clone()),r.width as usize*4);
            self.cover.set_paintable(Some(&texture));self.cover.set_visible(true);
        }else{self.cover.set_paintable(None::<&gtk::gdk::Paintable>);self.cover.set_visible(false);}
        *old=art;
    }
    fn colors(&self)->Option<Colors>{
        let art=self.art.borrow();let image=art.as_ref()?;
        let dark=gtk::Settings::default().is_some_and(|s|s.property::<bool>("gtk-application-prefer-dark-theme"));
        if dark{image.dark}else{image.light}
    }
    fn render(&self){
        self.disable_actions();
        let state=self.details.borrow();
        let Some(d)=state.as_ref()else{
            self.track.set_text("No track information");self.artist.set_text("");self.album.set_text("");self.playback.set_text("Unavailable");
            self.wave.set_interactive(false);self.wave.update(None,false,None);self.shuffle.set_label("Shuffle unavailable");self.repeat.set_label("Repeat unavailable");self.position.set_text("— / —");self.scale.set_value(0.0);self.toggle.set_label("Play");return;
        };
        self.track.set_text(if d.title.is_empty(){"Unknown title"}else{&d.title});
        self.artist.set_text(&d.artist);self.album.set_text(&d.album);self.playback.set_text(&d.status);
        let playing=d.status=="Playing";
        self.toggle.set_label(if playing{"Pause"}else{"Play"});
        self.toggle.set_sensitive(if playing{d.capabilities.pause}else{d.capabilities.play});
        self.previous.set_sensitive(d.capabilities.previous);self.next.set_sensitive(d.capabilities.next);
        self.shuffle.set_sensitive(d.shuffle.is_some());
        self.shuffle.set_label(match d.shuffle{Some(true)=>"Shuffle on",Some(false)=>"Shuffle off",None=>"Shuffle unavailable"});
        self.repeat.set_sensitive(d.repeat.is_some());
        self.repeat.set_label(match d.repeat{Some(Repeat::Off)=>"Repeat off",Some(Repeat::Track)=>"Repeat track",Some(Repeat::Playlist)=>"Repeat playlist",None=>"Repeat unavailable"});
        for (button,active) in [(&self.shuffle,d.shuffle==Some(true)),(&self.repeat,d.repeat.is_some_and(|r|r!=Repeat::Off))]{
            if active{button.add_css_class("atelier-control-active");}else{button.remove_css_class("atelier-control-active");}
        }
        if let (Some(track),Some(position))=(&d.track,d.position_us){
            if !self.draft.get(){self.scale.set_value(position as f64/track.length_us as f64*1000.0);}
            self.wave.set_interactive(d.capabilities.seek);
            self.wave.update(Some(self.scale.value()/1000.0),playing&&!self.draft.get(),self.colors());
            self.scale.set_sensitive(d.capabilities.seek);
            self.seek.set_sensitive(d.capabilities.seek&&self.draft.get());
            self.position.set_text(&format!("{}{} / {}",if self.draft.get(){"Seek to "}else{""},
                time(if self.draft.get(){target(track,self.scale.value())}else{position}),time(track.length_us)));
        }else{
            self.wave.set_interactive(false);
            self.wave.update(None,false,None);
            self.scale.set_value(0.0);
            self.position.set_text(&format!("{} / —",d.position_us.map(time).unwrap_or_else(||"—".into())));
        }
    }
    fn drag(&self,event:Drag){
        if matches!(event,Drag::Cancel){self.drag_track.borrow_mut().take();return;}
        if self.busy.get()||self.updating.get(){return;}
        let track=self.details.borrow().as_ref().filter(|d|d.capabilities.seek).and_then(|d|d.track.clone());
        let Some(track)=track else{self.drag_track.borrow_mut().take();return;};
        let fraction=match event{
            Drag::Begin(v)=>{*self.drag_track.borrow_mut()=Some(track);v}
            Drag::Move(v)|Drag::End(v)=>{
                if self.drag_track.borrow().as_ref()!=Some(&track){return;}
                if matches!(event,Drag::End(_)){self.drag_track.borrow_mut().take();}
                v
            }
            Drag::Cancel=>return,
        };
        self.draft.set(true);self.updating.set(true);
        self.scale.set_value(fraction*1000.0);
        self.render();self.updating.set(false);
    }
    fn action(&self,action:impl FnOnce(&Details)->Option<Action>){
        let selected=self.details.borrow().as_ref().and_then(|d|action(d).map(|action|(d.identity.clone(),action)));
        if let Some((identity,action))=selected{self.request(Request::Act(identity,action));}
    }
}
fn target(track:&Track,value:f64)->i64{((value.clamp(0.0,1000.0)/1000.0)*track.length_us as f64).round() as i64}

pub fn show(app:&gtk::Application){
    if let Some(window)=app.windows().into_iter().find(|w|w.widget_name()=="atelier-media"){window.present();return;}
    let window=gtk::ApplicationWindow::builder().application(app).title("Atelier · Media").default_width(440).default_height(420).build();
    window.set_widget_name("atelier-media");
    let content=gtk::Box::new(gtk::Orientation::Vertical,12);content.add_css_class("atelier-control-window");
    content.set_margin_top(24);content.set_margin_bottom(24);content.set_margin_start(24);content.set_margin_end(24);
    content.append(&label("Now playing","atelier-control-heading"));
    let choice=gtk::ComboBoxText::new();choice.set_sensitive(false);content.append(&choice);
    let card=gtk::Box::new(gtk::Orientation::Vertical,8);card.add_css_class("atelier-control-card");content.append(&card);
    let artwork_enabled=gtk::CheckButton::with_label("Show local cover art (read player-provided image files)");content.append(&artwork_enabled);
    let cover=gtk::Picture::new();cover.set_size_request(-1,160);cover.set_can_shrink(true);cover.set_content_fit(gtk::ContentFit::Contain);cover.set_visible(false);card.append(&cover);
    let track=label("Looking for media players…","atelier-control-title");card.append(&track);
    let artist=label("","body");card.append(&artist);
    let album=label("","dim-label");card.append(&album);
    let playback=label("","dim-label");card.append(&playback);
    let position=label("— / —","body");card.append(&position);
    let wave=Wave::new();card.append(&wave.widget);
    let scale=gtk::Scale::with_range(gtk::Orientation::Horizontal,0.0,1000.0,1.0);
    scale.set_draw_value(false);scale.set_size_request(-1,48);scale.add_css_class("atelier-expressive-slider");scale.set_tooltip_text(Some("Playback position draft; press Seek to apply"));
    scale.set_direction(gtk::TextDirection::Ltr);
    let keyboard=gtk::Expander::new(Some("Keyboard seek controls"));keyboard.set_child(Some(&scale));card.append(&keyboard);
    let seek=gtk::Button::with_label("Seek");seek.set_size_request(-1,48);card.append(&seek);
    let controls=gtk::Box::new(gtk::Orientation::Horizontal,8);
    let previous=gtk::Button::with_label("Previous");let toggle=gtk::Button::with_label("Play");let next=gtk::Button::with_label("Next");
    previous.add_css_class("atelier-control-tonal");next.add_css_class("atelier-control-tonal");toggle.add_css_class("atelier-control-active");toggle.set_hexpand(true);
    for button in [&previous,&toggle,&next]{button.set_size_request(-1,48);controls.append(button);}
    card.append(&controls);
    let modes=gtk::Box::new(gtk::Orientation::Horizontal,8);
    let shuffle=gtk::Button::with_label("Shuffle unavailable");let repeat=gtk::Button::with_label("Repeat unavailable");
    for b in [&shuffle,&repeat]{b.set_size_request(-1,48);b.set_hexpand(true);b.add_css_class("atelier-control-tonal");modes.append(b);}
    card.append(&modes);
    let status=label("Connecting to session media service…","dim-label");content.append(&status);
    let (requests,rx)=mpsc::sync_channel(1);let (tx,updates)=mpsc::sync_channel(1);
    let context=glib::MainContext::new();let worker_context=context.clone();
    std::thread::spawn(move||{let _=worker_context.with_thread_default(||worker(&worker_context,rx,tx));});
    let requests=RequestPort{sender:requests,context};
    let ui=Rc::new(Ui{choice,track,artist,album,playback,scale,position,seek,previous,toggle,next,shuffle,repeat,status,
        artwork_enabled,cover,wave,art:RefCell::new(None),
        names:RefCell::new(vec![]),details:RefCell::new(None),revision:Cell::new(0),busy:Cell::new(false),
        updating:Cell::new(false),draft:Cell::new(false),drag_track:RefCell::new(None),requests});
    ui.disable_actions();ui.artwork_enabled.set_sensitive(false);
    {let weak=Rc::downgrade(&ui);ui.wave.connect_drag(move|event|{if let Some(ui)=weak.upgrade(){ui.drag(event);}});}
    {let weak=Rc::downgrade(&ui);ui.shuffle.connect_clicked(move |_|{if let Some(ui)=weak.upgrade(){ui.action(|d|d.shuffle.map(Action::Shuffle));}});}
    {let weak=Rc::downgrade(&ui);ui.repeat.connect_clicked(move |_|{if let Some(ui)=weak.upgrade(){ui.action(|d|d.repeat.map(Action::Repeat));}});}
    {let weak=Rc::downgrade(&ui);ui.artwork_enabled.connect_toggled(move|button|{
        if let Some(ui)=weak.upgrade(){
            if !button.is_active(){ui.set_art(None);ui.wave.update(None,false,None);}
            ui.request(Request::Artwork(button.is_active()));
        }
    });}
    {let weak=Rc::downgrade(&ui);ui.choice.connect_changed(move|choice|{
        if let Some(ui)=weak.upgrade(){if !ui.updating.get(){if let Some(name)=choice.active_id(){ui.request(Request::Select(name.to_string()));}}}
    });}
    {let weak=Rc::downgrade(&ui);ui.scale.connect_value_changed(move|_|{
        if let Some(ui)=weak.upgrade(){
            if !ui.updating.get()&&!ui.busy.get()&&ui.details.borrow().as_ref().is_some_and(|d|d.capabilities.seek){
                ui.draft.set(true);ui.updating.set(true);ui.render();ui.updating.set(false);
            }
        }
    });}
    {let weak=Rc::downgrade(&ui);ui.seek.connect_clicked(move |_|{
        if let Some(ui)=weak.upgrade(){if ui.draft.get(){ui.action(|d|d.track.clone().map(|t|{let p=target(&t,ui.scale.value());Action::Seek(t,p)}));}}
    });}
    for (button,action) in [(&ui.previous,Action::Previous),(&ui.next,Action::Next)]{
        let weak=Rc::downgrade(&ui);button.connect_clicked(move |_|{if let Some(ui)=weak.upgrade(){ui.action(|_|Some(action.clone()));}});
    }
    {let weak=Rc::downgrade(&ui);ui.toggle.connect_clicked(move |_|{if let Some(ui)=weak.upgrade(){ui.action(|d|Some(if d.status=="Playing"{Action::Pause}else{Action::Play}));}});}
    let weak=window.downgrade();
    glib::timeout_add_local(Duration::from_millis(100),move||{
        if weak.upgrade().is_none(){return glib::ControlFlow::Break;}
        match updates.try_recv(){
            Ok(reply)=>ui.apply(reply),
            Err(mpsc::TryRecvError::Empty)=>{},
            Err(mpsc::TryRecvError::Disconnected)=>{ui.disable_actions();ui.choice.set_sensitive(false);ui.artwork_enabled.set_sensitive(false);ui.set_art(None);ui.wave.set_interactive(false);ui.wave.update(None,false,None);ui.status.set_text("Media worker stopped.");return glib::ControlFlow::Break;}
        }
        glib::ControlFlow::Continue
    });
    let scroll=gtk::ScrolledWindow::builder().child(&content).hscrollbar_policy(gtk::PolicyType::Never).build();
    window.set_child(Some(&scroll));window.present();
}
#[cfg(test)]
mod tests{
    #[test]
    fn clock_format(){assert_eq!(super::time(65_000_000),"1:05");assert_eq!(super::time(3661_000_000),"1:01:01");}
}
