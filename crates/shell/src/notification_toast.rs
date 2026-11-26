//! Optional layer-shell notification cards. No daemon ownership or replay.
use crate::notification_bus::{self,Request};
use crate::notification_privacy::{Guard,Gate};
use atelier_config::{Appearance, NotificationAppearance, NotificationPosition, NotificationStyle};
use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde::Deserialize;
use std::{cell::RefCell, rc::Rc, collections::HashMap, sync::mpsc, time::{Duration, Instant}};

#[cfg(test)]
const LIFETIME: Duration = Duration::from_secs(5);
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct Action { key: String, label: String }
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct Notice {
    id: u32, app: String, summary: String, body: String,
    #[serde(default)] revision: u64,
    #[serde(default)] icon: String,
    #[serde(default="normal_urgency")] urgency: u8,
    #[serde(default)] progress: Option<u8>,
    #[serde(default)] actions: Vec<Action>,
}
fn normal_urgency()->u8{1}
fn safe_icon(icon:&str)->&str{
    if !icon.is_empty() && icon.len()<=128 && icon.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'-'|b'_')){icon}else{"dialog-information-symbolic"}
}
impl Notice {
    fn sequence(&self) -> u64 { if self.revision == 0 { self.id as u64 } else { self.revision } }
    fn persistent(&self) -> bool { self.progress.is_some_and(|p| p < 100) || !self.actions.is_empty() || self.urgency==2 }
}
#[derive(Debug, Deserialize)]
struct Popup {
    schema: u32, instance: String, last_id: u32, dnd: bool, notice: Option<Notice>,
    #[serde(default)] latest_revision: u64,
    #[serde(default)] cards: Vec<Notice>,
    #[serde(default)] queued: usize,
}
impl Popup {
    fn watermark(&self) -> u64 { if self.latest_revision == 0 { self.last_id as u64 } else { self.latest_revision } }
    fn eligible(&self) -> Vec<&Notice> {
        if self.cards.is_empty() { self.notice.iter().collect() }
        else { self.cards.iter().collect() }
    }
}

struct Visible { notice: Notice, until: Instant }
#[derive(Default)]
struct Cursor { instance: Option<String>, watermark: u64, visible: Vec<Visible>, queued: usize }
impl Cursor {
    fn observe(&mut self, snapshot: &Popup, now: Instant, limit: usize, lifetime: Duration) {
        let latest = snapshot.watermark();
        if self.instance.as_deref() != Some(snapshot.instance.as_str()) || latest < self.watermark {
            self.instance = Some(snapshot.instance.clone());
            self.watermark = latest;
            self.visible.clear(); self.queued = 0;
            return;
        }
        let previous = self.watermark;
        self.watermark = latest;
        if snapshot.dnd { self.visible.clear(); self.queued = 0; return; }
        let eligible = snapshot.eligible();
        self.visible.retain(|v| eligible.iter().any(|n| n.id == v.notice.id));
        for notice in eligible {
            if notice.sequence() > latest || notice.id > snapshot.last_id { continue; }
            if let Some(old) = self.visible.iter_mut().find(|v| v.notice.id == notice.id) {
                if old.notice.sequence() != notice.sequence() {
                    if notice.sequence() > previous {
                        *old = Visible { notice: notice.clone(), until: now + lifetime };
                    }
                } else { old.notice = notice.clone(); }
            } else if notice.sequence() > previous {
                self.visible.push(Visible { notice: notice.clone(), until: now + lifetime });
            }
        }
        self.visible.sort_unstable_by_key(|v| std::cmp::Reverse(v.notice.sequence()));
        self.visible.truncate(limit);
        self.queued = snapshot.queued.saturating_add(snapshot.cards.len().saturating_sub(limit));
        self.expire(now);
    }
    fn expire(&mut self, now: Instant) {
        self.visible.retain(|v| v.notice.persistent() || now < v.until);
        if self.visible.is_empty() { self.queued = 0; }
    }
    fn cards(&self) -> Vec<Notice> { self.visible.iter().map(|v| v.notice.clone()).collect() }
}

fn snapshot(json:&str) -> Result<Popup, String> {
    let popup: Popup = serde_json::from_str(json).map_err(|error| error.to_string())?;
    if popup.schema != 1 || popup.instance.is_empty() || popup.cards.len() > 3 ||
        popup.eligible().iter().any(|n| n.actions.len() > 8 || n.progress.is_some_and(|p| p > 100)) {
        return Err("Invalid notification popup snapshot".into());
    }
    Ok(popup)
}
fn clipped(value: &str, length: usize) -> String { value.chars().take(length).collect() }
fn label(text: &str, class: &str, limit: usize) -> gtk::Label {
    let label = gtk::Label::new(Some(&clipped(text, limit)));
    label.set_xalign(0.0); label.set_wrap(true); label.set_wrap_mode(gtk::pango::WrapMode::WordChar); label.set_max_width_chars(36);
    if !class.is_empty() { label.add_css_class(class); }
    label
}
fn render(root: &gtk::Box, cards: &[Notice], instance: &str, queued: usize, requests: &mpsc::SyncSender<Request>, style: NotificationStyle, expansions: &HashMap<u32,bool>) {
    let island = style == NotificationStyle::Island;
    let compact = style == NotificationStyle::Compact;
    while let Some(child) = root.first_child() { root.remove(&child); }
    if cards.is_empty() { return; }
    if island {
        let pill = gtk::Label::new(Some("Atelier · Notifications"));
        pill.add_css_class("atelier-notification-pill"); root.append(&pill);
    }
    for notice in cards {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
        card.add_css_class("atelier-notification-card");
        card.set_widget_name(&format!("notice-{}",notice.id));
        if compact {card.add_css_class("atelier-notification-compact");card.set_spacing(3);}
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let app_name = label(&notice.app, "dim-label", 80); app_name.set_hexpand(true);
        let icon=gtk::Image::from_icon_name(safe_icon(&notice.icon));icon.set_pixel_size(18);header.append(&icon);
        header.append(&app_name);
        let close = gtk::Button::from_icon_name("window-close-symbolic"); close.add_css_class("flat");
        close.set_tooltip_text(Some("Dismiss notification"));
        let sender = requests.clone(); let instance = instance.to_owned(); let id = notice.id;
        let dismiss_instance = instance.clone();
        close.connect_clicked(move |button| {
            if sender.try_send(Request::Dismiss(dismiss_instance.clone(),id)).is_ok() { button.set_sensitive(false); }
        });
        header.append(&close); card.append(&header);
        card.append(&label(&notice.summary, "heading", 120));
        if !notice.body.is_empty() {
            let detail = label(&notice.body, "", 240);
            let expander = gtk::Expander::new(Some("Details"));
            expander.set_child(Some(&detail)); expander.set_expanded(expansions.get(&notice.id).copied().unwrap_or(!compact && (!island || cards.len()==1)));
            card.append(&expander);
        }
        if let Some(progress) = notice.progress {
            let bar = gtk::ProgressBar::new(); bar.set_fraction(f64::from(progress)/100.0);
            bar.set_show_text(true); bar.set_text(Some(&format!("{progress}%"))); card.append(&bar);
        }
        if notice.revision != 0 {
            for action in notice.actions.iter().take(8) {
                let button = gtk::Button::new();
                let action_label=label(&action.label,"",64);action_label.set_xalign(0.5);button.set_child(Some(&action_label));
                let sender = requests.clone(); let instance = instance.clone();
                let key = action.key.clone(); let revision = notice.revision;
                button.connect_clicked(move |button| {
                    if sender.try_send(Request::Invoke(instance.clone(),id,revision,key.clone())).is_ok() { button.set_sensitive(false); }
                });
                card.append(&button);
            }
        }
        root.append(&card);
    }
    if (island || compact) && queued > 0 { root.append(&label(&format!("{queued} more in inbox"), "dim-label", 80)); }
}

// Preserve expansion across replacements, but not across daemon instances.
fn expansion_state(root:&gtk::Box)->HashMap<u32,bool>{
    let mut result=HashMap::new();let mut next=root.first_child();
    while let Some(widget)=next {
        next=widget.next_sibling();
        let name=widget.widget_name();
        let Some(id)=name.strip_prefix("notice-").and_then(|s|s.parse::<u32>().ok()) else {continue;};
        let mut child=widget.first_child();
        while let Some(item)=child {
            child=item.next_sibling();
            if let Ok(expander)=item.downcast::<gtk::Expander>() {result.insert(id,expander.is_expanded());break;}
        }
    }
    result
}

// The common progress-only replacement need not recreate a card, disturb
// its focus/expansion, or recreate its buttons. Action-bearing replacements
// are rebuilt because their callbacks must carry the current revision.
fn patch_progress(root:&gtk::Box,previous:&[Notice],next:&[Notice])->bool{
    if previous.len()!=next.len() || next.is_empty(){return false;}
    if !previous.iter().zip(next).all(|(a,b)|a==b ||
        (a.id==b.id && a.app==b.app && a.summary==b.summary && a.body==b.body && a.icon==b.icon && a.urgency==b.urgency &&
        a.actions.is_empty() && b.actions.is_empty() && a.progress.is_some() && b.progress.is_some())){return false;}
    let mut bars=Vec::new();
    for (a,b) in previous.iter().zip(next){
        if a==b{continue;}
        let mut child=root.first_child();let mut found=None;
        while let Some(widget)=child{
            child=widget.next_sibling();
            if widget.widget_name().as_str()!=format!("notice-{}",b.id){continue;}
            let mut item=widget.first_child();
            while let Some(current)=item{
                item=current.next_sibling();
                if let Ok(bar)=current.downcast::<gtk::ProgressBar>(){found=Some(bar);break;}
            }
            break;
        }
        let Some(bar)=found else{return false;};
        bars.push((bar,b.progress.unwrap()));
    }
    for (bar,progress) in bars{bar.set_fraction(f64::from(progress)/100.0);bar.set_text(Some(&format!("{progress}%")));}
    true
}

// One presenter follows a single selected output. Never duplicate actions on
// several outputs. Missing connectors fall back to the panel, then the first
// available monitor; the original preference is retained for hotplug.
#[derive(Clone,PartialEq,Eq)]
struct Output { monitor:gdk::Monitor,width:i32,height:i32 }
fn select_index<'a>(connectors:impl IntoIterator<Item=Option<&'a str>>,preferred:&str,panel:Option<usize>)->Option<usize>{
    let names:Vec<_>=connectors.into_iter().collect();
    if names.is_empty(){return None;}
    if !preferred.is_empty(){
        if let Some(index)=names.iter().position(|name|*name==Some(preferred)){return Some(index);}
    }
    Some(panel.filter(|index|*index<names.len()).unwrap_or(0))
}
fn select_output(panel:&gtk::ApplicationWindow,connector:&str)->Option<Output>{
    let display=gdk::Display::default()?;
    let monitors=display.monitors();
    let available:Vec<_>=(0..monitors.n_items()).filter_map(|i|monitors.item(i).and_downcast::<gdk::Monitor>()).collect();
    let panel_index=panel.surface().and_then(|surface|display.monitor_at_surface(&surface))
        .and_then(|monitor|available.iter().position(|m|m==&monitor));
    let connectors:Vec<_>=available.iter().map(|monitor|monitor.connector()).collect();
    let index=select_index(connectors.iter().map(|name|name.as_deref()),connector,panel_index)?;
    let monitor=available[index].clone();
    let geometry=monitor.geometry();
    Some(Output{monitor,width:geometry.width(),height:geometry.height()})
}
#[derive(Debug,PartialEq,Eq)]
struct Geometry { width:i32,height:i32,top:i32,margin:i32 }
fn geometry(preferred:u32,panel_height:u32,output:Option<(i32,i32)>)->Geometry{
    let (width,height)=output.unwrap_or((1024,768));
    let width=width.max(1);let height=height.max(1);
    let margin=12.min((width-1)/2).min((height-1)/2);
    let top=(panel_height as i32+12).min((height/3).max(0));
    Geometry{width:(preferred as i32).min((width-2*margin).max(1)),height:(height-top-margin).max(1),top,margin}
}
fn apply_layout(window:&gtk::ApplicationWindow,scroll:&gtk::ScrolledWindow,revealer:&gtk::Revealer,appearance:&Appearance,island:bool,output:Option<&Output>){
    let n=&appearance.notifications;
    let style=n.style_for_launch(island);
    let position=n.position_for_style(style);
    let g=geometry(n.width,appearance.panel_height,output.map(|o|(o.width,o.height)));
    window.set_anchor(Edge::Left,position==NotificationPosition::TopLeft);
    window.set_anchor(Edge::Right,position==NotificationPosition::TopRight);
    window.set_margin(Edge::Left,g.margin);window.set_margin(Edge::Right,g.margin);
    window.set_margin(Edge::Top,g.top);
    window.set_default_size(g.width,-1);
    scroll.set_min_content_width(g.width);
    scroll.set_max_content_height(g.height);
    revealer.set_transition_duration(n.transition_ms(appearance.motion));
}
fn limit(n:&NotificationAppearance,island:bool)->usize{
    if n.style_for_launch(island)==NotificationStyle::Floating{1}else{n.max_cards as usize}
}

pub fn start(app: &gtk::Application, panel: &gtk::ApplicationWindow, island: bool, live:Rc<RefCell<Appearance>>) {
    let window = gtk::ApplicationWindow::builder().application(app).title("Atelier notifications").decorated(false).default_width(340).build();
    window.add_css_class("atelier-notification-layer");
    window.init_layer_shell(); window.set_namespace(Some("atelier-notification-toast")); window.set_layer(Layer::Overlay);
    window.set_anchor(Edge::Top,true);
    window.set_keyboard_mode(KeyboardMode::None); window.set_exclusive_zone(0);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
    root.set_margin_top(4); root.set_margin_bottom(4); root.set_margin_start(8); root.set_margin_end(8);
    let scroll=gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vscrollbar_policy(gtk::PolicyType::Automatic).propagate_natural_height(true).child(&root).build();
    let revealer = gtk::Revealer::new(); revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    revealer.set_child(Some(&scroll)); window.set_child(Some(&revealer));
    let mut applied=live.borrow().clone();
    let mut output=select_output(panel,&applied.notifications.output);
    window.set_monitor(output.as_ref().map(|o|&o.monitor));
    apply_layout(&window,&scroll,&revealer,&applied,island,output.as_ref());
    let mut next_output_check=Instant::now();
    let (requests,receive,watch)=notification_bus::start("Popup",snapshot);
    let panel = panel.downgrade(); let mut cursor = Cursor::default();
    let mut privacy=Guard::default();let mut gate=Gate::default();
    let mut rendered = Vec::new(); let mut rendered_instance = None; let mut rendered_queued = 0;
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let Some(panel)=panel.upgrade()else{window.close();return glib::ControlFlow::Break;};
        let _=&watch;
        let now = Instant::now();
        let current=live.borrow();
        let mut layout_changed=applied.notifications!=current.notifications || applied.motion!=current.motion || applied.panel_height!=current.panel_height;
        if now>=next_output_check || applied.notifications.output!=current.notifications.output{
            next_output_check=now+Duration::from_secs(1);
            let next=select_output(&panel,&current.notifications.output);
            if output!=next{
                if output.as_ref().map(|o|&o.monitor)!=next.as_ref().map(|o|&o.monitor){window.set_monitor(next.as_ref().map(|o|&o.monitor));}
                output=next;layout_changed=true;
            }
        }
        if layout_changed{apply_layout(&window,&scroll,&revealer,&current,island,output.as_ref());applied=current.clone();}
        drop(current);
        let prefs=&applied.notifications;
        let style=prefs.style_for_launch(island);
        let card_limit=limit(prefs,island);
        let permitted=privacy.permits(prefs.hide_when_session_unavailable,now);
        if gate.update(permitted,now){
            cursor=Cursor::default();
            // Privacy changes hide immediately, never animate old content
            // out. Resume establishes a fresh no-replay watermark.
            revealer.set_reveal_child(false);window.set_visible(false);
            if permitted{let _=requests.try_send(Request::Refresh);}
        }
        let mut action_reply = None;
        match receive.try_recv() {
            Ok(reply) if gate.accepts(reply.started) => match reply.snapshot {
                Ok(state)=>{cursor.observe(&state,now,card_limit,Duration::from_secs(u64::from(prefs.timeout_seconds)));action_reply=reply.action_error;}
                Err(_)=>cursor=Cursor::default(),
            },
            Ok(_) => {},
            Err(mpsc::TryRecvError::Disconnected) => { window.close(); return glib::ControlFlow::Break; }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        cursor.expire(now);
        cursor.visible.truncate(card_limit);
        let cards = cursor.cards();
        if rendered != cards || rendered_instance != cursor.instance || rendered_queued != cursor.queued || action_reply.is_some() || layout_changed {
            let same_instance=rendered_instance==cursor.instance;
            let patched=same_instance && !layout_changed && action_reply.is_none() && rendered_queued==cursor.queued && patch_progress(&root,&rendered,&cards);
            if !patched{
                let expansions=if same_instance{expansion_state(&root)}else{HashMap::new()};
                render(&root,&cards,cursor.instance.as_deref().unwrap_or(""),cursor.queued,&requests,style,&expansions);
            }
            if action_reply.is_some() && !cards.is_empty() {
                root.append(&label("Action unavailable. Refreshing notification state.", "dim-label", 80));
            }
            rendered = cards; rendered_instance = cursor.instance.clone(); rendered_queued = cursor.queued;
            if rendered.is_empty() { revealer.set_reveal_child(false); window.set_visible(false); }
            else if !window.is_visible() { window.present(); revealer.set_reveal_child(true); }
        }
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn notice(id:u32, revision:u64, progress:Option<u8>) -> Notice {
        Notice { id, revision, progress, icon:String::new(),urgency:1,actions:vec![], app:"app".into(),summary:"title".into(),body:"".into() }
    }
    fn state(instance:&str,revision:u64,dnd:bool,cards:Vec<Notice>) -> Popup {
        Popup {schema:1,instance:instance.into(),last_id:10,dnd,notice:None,latest_revision:revision,cards,queued:0}
    }
    #[test]
    fn connector_selection_falls_back_and_recovers_after_hotplug(){
        let connected=[Some("eDP-1"),Some("DP-2")];
        assert_eq!(select_index(connected,"DP-2",Some(0)),Some(1));
        assert_eq!(select_index(connected,"",Some(1)),Some(1));
        assert_eq!(select_index([Some("eDP-1")],"DP-2",Some(0)),Some(0));
        assert_eq!(select_index(connected,"DP-2",Some(0)),Some(1));
        assert_eq!(select_index([None],"DP-2",Some(4)),Some(0));
        assert_eq!(select_index([],"DP-2",None),None);
    }
    #[test]
    fn geometry_respects_logical_output_bounds(){
        assert_eq!(geometry(480,48,Some((1920,1080))),Geometry{width:480,height:1008,top:60,margin:12});
        assert_eq!(geometry(480,48,Some((320,240))),Geometry{width:296,height:168,top:60,margin:12});
        let tiny=geometry(340,96,Some((10,10)));
        assert_eq!(tiny.width,2);assert_eq!(tiny.height,3);
    }
    #[test]
    fn icon_names_are_not_file_paths(){
        assert_eq!(safe_icon("emblem-ok-symbolic"),"emblem-ok-symbolic");
        for invalid in ["","/tmp/image.png","../image","file:icon"]{assert_eq!(safe_icon(invalid),"dialog-information-symbolic");}
    }
    #[test]
    fn no_replay_and_live_replacement() {
        let now=Instant::now();let mut c=Cursor::default();
        c.observe(&state("a",1,false,vec![notice(1,1,None)]),now,3,LIFETIME);
        assert!(c.cards().is_empty());
        c.observe(&state("a",2,false,vec![notice(1,2,Some(30))]),now,3,LIFETIME);
        assert_eq!(c.cards()[0].progress,Some(30));
        c.expire(now+Duration::from_secs(20));assert_eq!(c.cards().len(),1);
        c.observe(&state("a",3,false,vec![notice(1,3,Some(100))]),now,3,LIFETIME);
        c.expire(now+LIFETIME);assert!(c.cards().is_empty());
        c.observe(&state("a",3,false,vec![notice(1,3,Some(100))]),now,3,LIFETIME);
        assert!(c.cards().is_empty());
    }
    #[test]
    fn configurable_timeout_and_stack_limits(){
        let now=Instant::now();let mut c=Cursor::default();
        let lifetime=Duration::from_secs(2);
        c.observe(&state("a",1,false,vec![]),now,2,lifetime);
        c.observe(&state("a",4,false,vec![notice(4,4,None),notice(3,3,None),notice(2,2,None)]),now,2,lifetime);
        assert_eq!(c.cards().len(),2);assert_eq!(c.queued,1);
        c.expire(now+Duration::from_secs(1));assert_eq!(c.cards().len(),2);
        c.expire(now+lifetime);assert!(c.cards().is_empty());
        let mut prefs=NotificationAppearance::default();
        assert_eq!(limit(&prefs,false),1);assert_eq!(limit(&prefs,true),3);
        prefs.style=NotificationStyle::Compact;prefs.max_cards=2;
        assert_eq!(limit(&prefs,false),2);
    }
    #[test]
    fn stack_and_dnd_never_replay_old_cards() {
        let now=Instant::now();let mut c=Cursor::default();
        c.observe(&state("a",1,false,vec![]),now,3,LIFETIME);
        c.observe(&state("a",4,false,vec![notice(4,4,None),notice(3,3,None),notice(2,2,None)]),now,3,LIFETIME);
        assert_eq!(c.cards().iter().map(|n|n.id).collect::<Vec<_>>(),vec![4,3,2]);
        c.observe(&state("a",5,true,vec![]),now,3,LIFETIME);
        c.observe(&state("a",5,false,vec![notice(5,5,None)]),now,3,LIFETIME);
        assert!(c.cards().is_empty());
        c.observe(&state("b",6,false,vec![notice(6,6,None)]),now,3,LIFETIME);
        assert!(c.cards().is_empty());
    }
}
