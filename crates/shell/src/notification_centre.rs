//! Optional notification inbox. Never starts or replaces a notification daemon.
use atelier_config::component;
use gtk::{glib, prelude::*};
use serde::Deserialize;
use std::{cell::RefCell, rc::Rc, process::Command, sync::mpsc, time::Duration};

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Notice { id: u32, app: String, summary: String, body: String, #[serde(default)] progress: Option<u8> }
#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Snapshot { schema: u32, instance: String, dnd: bool, active: Vec<Notice>, history: Vec<Notice> }
enum Request { Dismiss(String, u32), Clear(String), Dnd(String, bool), Refresh }
struct Reply { snapshot: Result<Snapshot, String>, action_error: Option<String> }

pub(crate) fn call(args: &[&str]) -> Result<String, String> {
    let helper = component("atelier-notifications");
    let output = Command::new("timeout").args(["--kill-after=1s", "3s"]).arg(helper.get_program())
        .args(args).output().map_err(|error| error.to_string())?;
    if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()); }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}
fn snapshot() -> Result<Snapshot, String> {
    let state: Snapshot = serde_json::from_str(&call(&["list"])?).map_err(|error| error.to_string())?;
    if state.schema != 1 || state.instance.is_empty() || state.active.len() > 128 || state.history.len() > 100 {
        return Err("Unsupported notification snapshot".into());
    }
    Ok(state)
}

fn format_notices(notices: &[Notice]) -> String {
    if notices.is_empty() { return "None".into(); }
    notices.iter().map(|notice| {
        let progress = notice.progress.filter(|v| *v <= 100).map(|v| format!(" · {v}%")).unwrap_or_default();
        format!("{} · {}{progress}\n{}\n{}", notice.id, notice.app, notice.summary, notice.body)
    })
        .collect::<Vec<_>>().join("\n\n")
}

pub fn show(app: &gtk::Application) {
    if let Some(window) = app.windows().into_iter().find(|window| window.widget_name() == "atelier-inbox") {
        window.present(); return;
    }
    let window = gtk::ApplicationWindow::builder().application(app).title("Atelier · Notifications").default_width(570).default_height(540).build();
    window.set_widget_name("atelier-inbox");
    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.set_margin_top(20); root.set_margin_bottom(20); root.set_margin_start(20); root.set_margin_end(20);
    let title = gtk::Label::new(Some("Notification centre")); title.set_xalign(0.0); title.add_css_class("title-1"); root.append(&title);
    let status = gtk::Label::new(Some("Reading Atelier notification inbox…")); status.set_wrap(true); status.set_xalign(0.0); root.append(&status);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let choice = gtk::ComboBoxText::new(); choice.set_hexpand(true);
    let dismiss = gtk::Button::with_label("Dismiss"); dismiss.set_sensitive(false);
    controls.append(&choice); controls.append(&dismiss); root.append(&controls);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let clear = gtk::Button::with_label("Clear closed history"); clear.set_sensitive(false);
    let refresh = gtk::Button::with_label("Refresh");
    let dnd = gtk::Button::with_label("Do Not Disturb"); dnd.set_sensitive(false);
    actions.append(&clear); actions.append(&refresh); actions.append(&dnd); root.append(&actions);
    let text = gtk::TextView::new(); text.set_editable(false); text.set_cursor_visible(false); text.set_wrap_mode(gtk::WrapMode::WordChar);
    text.buffer().set_text("Active notifications and closed history appear here.");
    let scroll = gtk::ScrolledWindow::builder().vexpand(true).child(&text).build(); root.append(&scroll);

    let (requests, incoming) = mpsc::sync_channel::<Request>(1);
    let (outgoing, replies) = mpsc::sync_channel::<Reply>(2);
    std::thread::spawn(move || {
        let mut action_error = None;
        loop {
            if outgoing.send(Reply { snapshot: snapshot(), action_error: action_error.take() }).is_err() { break; }
            let result = match incoming.recv_timeout(Duration::from_secs(3)) {
                Ok(Request::Dismiss(instance, id)) => call(&["dismiss", &instance, &id.to_string()]),
                Ok(Request::Clear(instance)) => call(&["clear-history", &instance]),
                Ok(Request::Dnd(instance, enabled)) => call(&["dnd", &instance, if enabled { "on" } else { "off" }]),
                Ok(Request::Refresh) | Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            action_error = result.err();
        }
    });
    let state: Rc<RefCell<Option<Snapshot>>> = Rc::new(RefCell::new(None));
    let current = Rc::clone(&state); let sender = requests.clone(); let selected = choice.clone(); let message = status.clone();
    dismiss.connect_clicked(move |_| {
        let Some(id) = selected.active_id().and_then(|id| id.parse::<u32>().ok()) else { return; };
        if let Some(snapshot) = current.borrow().as_ref() {
            if sender.try_send(Request::Dismiss(snapshot.instance.clone(), id)).is_err() { message.set_text("Notification service busy."); }
        }
    });
    let current = Rc::clone(&state); let sender = requests.clone(); let message = status.clone();
    clear.connect_clicked(move |_| {
        if let Some(snapshot) = current.borrow().as_ref() {
            if sender.try_send(Request::Clear(snapshot.instance.clone())).is_err() { message.set_text("Notification service busy."); }
        }
    });
    let current = Rc::clone(&state); let sender = requests.clone(); let message = status.clone();
    dnd.connect_clicked(move |_| {
        if let Some(snapshot) = current.borrow().as_ref() {
            if sender.try_send(Request::Dnd(snapshot.instance.clone(), !snapshot.dnd)).is_err() { message.set_text("Notification service busy."); }
        }
    });
    let sender = requests.clone();
    refresh.connect_clicked(move |_| { let _ = sender.try_send(Request::Refresh); });

    let weak = window.downgrade();
    glib::timeout_add_local(Duration::from_millis(150), move || {
        if weak.upgrade().is_none() { return glib::ControlFlow::Break; }
        let _ = &requests;
        while let Ok(reply) = replies.try_recv() {
            match reply.snapshot {
                Ok(snapshot) => {
                    if state.borrow().as_ref() != Some(&snapshot) {
                        let old_id = if state.borrow().as_ref().is_some_and(|old| old.instance == snapshot.instance) { choice.active_id() } else { None };
                        choice.remove_all();
                        for notice in &snapshot.active {
                            let label: String = notice.summary.chars().take(80).collect();
                            choice.append(Some(&notice.id.to_string()), &format!("{} · {label}", notice.id));
                        }
                        if !old_id.as_ref().is_some_and(|id| choice.set_active_id(Some(id.as_str()))) && !snapshot.active.is_empty() { choice.set_active(Some(0)); }
                        text.buffer().set_text(&format!("ACTIVE ({})\n{}\n\nCLOSED HISTORY ({})\n{}", snapshot.active.len(), format_notices(&snapshot.active), snapshot.history.len(), format_notices(&snapshot.history)));
                        dismiss.set_sensitive(!snapshot.active.is_empty()); clear.set_sensitive(!snapshot.history.is_empty());
                        dnd.set_sensitive(true); dnd.set_label(if snapshot.dnd { "Do Not Disturb: On" } else { "Do Not Disturb: Off" });
                        *state.borrow_mut() = Some(snapshot);
                    }
                    status.set_text(reply.action_error.as_deref().unwrap_or("Atelier inbox · content stays in session memory"));
                }
                Err(error) => {
                    *state.borrow_mut() = None;
                    choice.remove_all(); dismiss.set_sensitive(false); clear.set_sensitive(false); dnd.set_sensitive(false);
                    text.buffer().set_text("No Atelier inbox available. Your existing notification service is untouched.");
                    status.set_text(&format!("Atelier notification service unavailable: {error}"));
                }
            }
        }
        glib::ControlFlow::Continue
    });
    window.set_child(Some(&root)); window.present();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn body_is_plain_text_and_history_empty_is_explicit() {
        let notice = Notice { id: 1, app: "Test".into(), summary: "<b>title</b>".into(), body: "text".into(), progress: Some(72) };
        let rendered = format_notices(&[notice]);
        assert!(rendered.contains("<b>title</b>"));
        assert!(rendered.contains("72%"));
        assert_eq!(format_notices(&[]), "None");
    }
}
