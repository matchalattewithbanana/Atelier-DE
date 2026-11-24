//! Optional non-interactive layer-shell toast; no daemon ownership or replay.
use gtk::{glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde::Deserialize;
use std::{sync::mpsc, time::{Duration, Instant}};

#[derive(Debug, Deserialize)]
struct Notice { id: u32, app: String, summary: String, body: String }
#[derive(Debug, Deserialize)]
struct Popup { schema: u32, instance: String, last_id: u32, dnd: bool, notice: Option<Notice> }

#[derive(Default)]
struct Cursor { instance: Option<String>, last_id: u32 }
impl Cursor {
    fn observe(&mut self, snapshot: &Popup) -> bool {
        if self.instance.as_deref() != Some(snapshot.instance.as_str()) {
            self.instance = Some(snapshot.instance.clone());
            self.last_id = snapshot.last_id;
            return false; // Never replay notices from before the presenter started.
        }
        let is_new = !snapshot.dnd && snapshot.notice.as_ref().is_some_and(|n| n.id > self.last_id && n.id <= snapshot.last_id);
        self.last_id = self.last_id.max(snapshot.last_id);
        is_new
    }
}

fn snapshot() -> Result<Popup, String> {
    let popup: Popup = serde_json::from_str(&crate::notification_centre::call(&["popup"])?).map_err(|error| error.to_string())?;
    if popup.schema != 1 || popup.instance.is_empty() { return Err("Invalid notification popup snapshot".into()); }
    Ok(popup)
}

pub fn start(app: &gtk::Application, panel: &gtk::ApplicationWindow) {
    let window = gtk::ApplicationWindow::builder().application(app).title("Atelier notification").decorated(false).default_width(340).build();
    window.init_layer_shell(); window.set_namespace(Some("atelier-notification-toast")); window.set_layer(Layer::Overlay);
    window.set_anchor(Edge::Top, true); window.set_anchor(Edge::Right, true);
    window.set_margin(Edge::Top, 12); window.set_margin(Edge::Right, 12);
    window.set_keyboard_mode(KeyboardMode::None); window.set_exclusive_zone(0);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
    box_.set_margin_top(16); box_.set_margin_bottom(16); box_.set_margin_start(16); box_.set_margin_end(16);
    box_.add_css_class("atelier-toast");
    let app_name = gtk::Label::new(None); app_name.set_xalign(0.0); app_name.add_css_class("dim-label");
    let title = gtk::Label::new(None); title.set_xalign(0.0); title.add_css_class("heading");
    let body = gtk::Label::new(None); body.set_xalign(0.0);
    for label in [&app_name, &title, &body] { label.set_wrap(true); label.set_max_width_chars(36); box_.append(label); }
    window.set_child(Some(&box_));

    let (send, receive) = mpsc::sync_channel(1);
    std::thread::spawn(move || loop {
        if send.send(snapshot()).is_err() { break; }
        std::thread::sleep(Duration::from_secs(1));
    });
    let panel = panel.downgrade();
    let mut cursor = Cursor::default();
    let mut visible: Option<(u32, Instant)> = None;
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if panel.upgrade().is_none() { window.close(); return glib::ControlFlow::Break; }
        match receive.try_recv() {
            Ok(Ok(state)) => {
                let changed_instance = cursor.instance.as_deref() != Some(state.instance.as_str());
                let new_notice = cursor.observe(&state);
                if changed_instance || state.dnd || visible.is_some_and(|(id, _)| state.notice.as_ref().is_none_or(|notice| notice.id != id)) {
                    window.set_visible(false); visible = None;
                }
                if new_notice {
                    if let Some(notice) = state.notice {
                        app_name.set_text(&notice.app.chars().take(80).collect::<String>());
                        title.set_text(&notice.summary.chars().take(120).collect::<String>());
                        body.set_text(&notice.body.chars().take(240).collect::<String>());
                        visible = Some((notice.id, Instant::now() + Duration::from_secs(5)));
                        window.present();
                    }
                }
            }
            Ok(Err(_)) => { window.set_visible(false); visible = None; cursor = Cursor::default(); }
            Err(mpsc::TryRecvError::Disconnected) => { window.close(); return glib::ControlFlow::Break; }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if visible.is_some_and(|(_, deadline)| Instant::now() >= deadline) { window.set_visible(false); visible = None; }
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(instance: &str, last_id: u32, dnd: bool) -> Popup {
        Popup { schema: 1, instance: instance.into(), last_id, dnd,
            notice: Some(Notice { id: last_id, app: "".into(), summary: "".into(), body: "".into() }) }
    }
    #[test]
    fn only_new_unsuppressed_notices_show() {
        let mut cursor = Cursor::default();
        assert!(!cursor.observe(&state("a", 1, false)));
        assert!(cursor.observe(&state("a", 2, false)));
        assert!(!cursor.observe(&state("a", 2, false)));
        assert!(!cursor.observe(&state("a", 3, true)));
        assert!(!cursor.observe(&state("a", 3, false)));
        assert!(!cursor.observe(&state("b", 1, false)));
        assert!(cursor.observe(&state("b", 2, false)));
    }
}
