//! Optional layer-shell notification cards. No daemon ownership or replay.
use gtk::{glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde::Deserialize;
use std::{sync::mpsc, time::{Duration, Instant}};

const LIFETIME: Duration = Duration::from_secs(5);
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct Action { key: String, label: String }
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct Notice {
    id: u32, app: String, summary: String, body: String,
    #[serde(default)] revision: u64,
    #[serde(default)] progress: Option<u8>,
    #[serde(default)] actions: Vec<Action>,
}
impl Notice {
    fn sequence(&self) -> u64 { if self.revision == 0 { self.id as u64 } else { self.revision } }
    fn persistent(&self) -> bool { self.progress.is_some_and(|p| p < 100) || !self.actions.is_empty() }
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
    fn observe(&mut self, snapshot: &Popup, now: Instant, limit: usize) {
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
                        *old = Visible { notice: notice.clone(), until: now + LIFETIME };
                    }
                } else { old.notice = notice.clone(); }
            } else if notice.sequence() > previous {
                self.visible.push(Visible { notice: notice.clone(), until: now + LIFETIME });
            }
        }
        self.visible.sort_unstable_by_key(|v| std::cmp::Reverse(v.notice.sequence()));
        self.visible.truncate(limit);
        self.queued = snapshot.queued;
        self.expire(now);
    }
    fn expire(&mut self, now: Instant) {
        self.visible.retain(|v| v.notice.persistent() || now < v.until);
        if self.visible.is_empty() { self.queued = 0; }
    }
    fn cards(&self) -> Vec<Notice> { self.visible.iter().map(|v| v.notice.clone()).collect() }
}

fn snapshot() -> Result<Popup, String> {
    let popup: Popup = serde_json::from_str(&crate::notification_centre::call(&["popup"])?).map_err(|error| error.to_string())?;
    if popup.schema != 1 || popup.instance.is_empty() || popup.cards.len() > 3 ||
        popup.eligible().iter().any(|n| n.actions.len() > 8 || n.progress.is_some_and(|p| p > 100)) {
        return Err("Invalid notification popup snapshot".into());
    }
    Ok(popup)
}
enum Request { Invoke(String,u32,u64,String), Dismiss(String,u32) }
fn clipped(value: &str, length: usize) -> String { value.chars().take(length).collect() }
fn label(text: &str, class: &str, limit: usize) -> gtk::Label {
    let label = gtk::Label::new(Some(&clipped(text, limit)));
    label.set_xalign(0.0); label.set_wrap(true); label.set_max_width_chars(36);
    if !class.is_empty() { label.add_css_class(class); }
    label
}
fn render(root: &gtk::Box, cards: &[Notice], instance: &str, queued: usize, requests: &mpsc::SyncSender<Request>, island: bool) {
    while let Some(child) = root.first_child() { root.remove(&child); }
    if cards.is_empty() { return; }
    if island {
        let pill = gtk::Label::new(Some("Atelier · Notifications"));
        pill.add_css_class("atelier-notification-pill"); root.append(&pill);
    }
    for notice in cards {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
        card.add_css_class("atelier-notification-card");
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let app_name = label(&notice.app, "dim-label", 80); app_name.set_hexpand(true);
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
            expander.set_child(Some(&detail)); expander.set_expanded(!island || cards.len()==1);
            card.append(&expander);
        }
        if let Some(progress) = notice.progress {
            let bar = gtk::ProgressBar::new(); bar.set_fraction(f64::from(progress)/100.0);
            bar.set_show_text(true); bar.set_text(Some(&format!("{progress}%"))); card.append(&bar);
        }
        if notice.revision != 0 {
            for action in notice.actions.iter().take(8) {
                let button = gtk::Button::with_label(&clipped(&action.label, 64));
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
    if island && queued > 0 { root.append(&label(&format!("{queued} more in inbox"), "dim-label", 80)); }
}

pub fn start(app: &gtk::Application, panel: &gtk::ApplicationWindow, island: bool) {
    let window = gtk::ApplicationWindow::builder().application(app).title("Atelier notifications").decorated(false).default_width(340).build();
    window.add_css_class("atelier-notification-layer");
    window.init_layer_shell(); window.set_namespace(Some("atelier-notification-toast")); window.set_layer(Layer::Overlay);
    window.set_anchor(Edge::Top,true);
    if !island { window.set_anchor(Edge::Right,true); window.set_margin(Edge::Right,12); }
    window.set_margin(Edge::Top,88); window.set_keyboard_mode(KeyboardMode::None); window.set_exclusive_zone(0);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
    root.set_margin_top(4); root.set_margin_bottom(4); root.set_margin_start(8); root.set_margin_end(8);
    let revealer = gtk::Revealer::new(); revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    revealer.set_transition_duration(160); revealer.set_child(Some(&root)); window.set_child(Some(&revealer));
    let (send, receive) = mpsc::sync_channel(1);
    let (requests, incoming) = mpsc::sync_channel::<Request>(1);
    std::thread::spawn(move || loop {
        if send.send(snapshot()).is_err() { break; }
        match incoming.recv_timeout(Duration::from_millis(500)) {
            Ok(Request::Invoke(instance,id,revision,key)) => {
                let _ = crate::notification_centre::call(&["invoke",&instance,&id.to_string(),&revision.to_string(),&key]);
            }
            Ok(Request::Dismiss(instance,id)) => {
                let _ = crate::notification_centre::call(&["dismiss",&instance,&id.to_string()]);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    });
    let panel = panel.downgrade(); let mut cursor = Cursor::default();
    let mut rendered = Vec::new(); let mut rendered_instance = None; let mut rendered_queued = 0;
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if panel.upgrade().is_none() { window.close(); return glib::ControlFlow::Break; }
        let now = Instant::now();
        match receive.try_recv() {
            Ok(Ok(state)) => cursor.observe(&state,now,if island {3} else {1}),
            Ok(Err(_)) => cursor = Cursor::default(),
            Err(mpsc::TryRecvError::Disconnected) => { window.close(); return glib::ControlFlow::Break; }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        cursor.expire(now);
        let cards = cursor.cards();
        if rendered != cards || rendered_instance != cursor.instance || rendered_queued != cursor.queued {
            render(&root,&cards,cursor.instance.as_deref().unwrap_or(""),cursor.queued,&requests,island);
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
        Notice { id, revision, progress, actions:vec![], app:"app".into(),summary:"title".into(),body:"".into() }
    }
    fn state(instance:&str,revision:u64,dnd:bool,cards:Vec<Notice>) -> Popup {
        Popup {schema:1,instance:instance.into(),last_id:10,dnd,notice:None,latest_revision:revision,cards,queued:0}
    }
    #[test]
    fn no_replay_and_live_replacement() {
        let now=Instant::now();let mut c=Cursor::default();
        c.observe(&state("a",1,false,vec![notice(1,1,None)]),now,3);
        assert!(c.cards().is_empty());
        c.observe(&state("a",2,false,vec![notice(1,2,Some(30))]),now,3);
        assert_eq!(c.cards()[0].progress,Some(30));
        c.expire(now+Duration::from_secs(20));assert_eq!(c.cards().len(),1);
        c.observe(&state("a",3,false,vec![notice(1,3,Some(100))]),now,3);
        c.expire(now+LIFETIME);assert!(c.cards().is_empty());
        c.observe(&state("a",3,false,vec![notice(1,3,Some(100))]),now,3);
        assert!(c.cards().is_empty());
    }
    #[test]
    fn stack_and_dnd_never_replay_old_cards() {
        let now=Instant::now();let mut c=Cursor::default();
        c.observe(&state("a",1,false,vec![]),now,3);
        c.observe(&state("a",4,false,vec![notice(4,4,None),notice(3,3,None),notice(2,2,None)]),now,3);
        assert_eq!(c.cards().iter().map(|n|n.id).collect::<Vec<_>>(),vec![4,3,2]);
        c.observe(&state("a",5,true,vec![]),now,3);
        c.observe(&state("a",5,false,vec![notice(5,5,None)]),now,3);
        assert!(c.cards().is_empty());
        c.observe(&state("b",6,false,vec![notice(6,6,None)]),now,3);
        assert!(c.cards().is_empty());
    }
}
