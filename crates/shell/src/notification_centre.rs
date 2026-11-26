//! Optional notification inbox. Never starts or replaces a notification daemon.
use atelier_config::component;
use gtk::{glib, prelude::*};
use serde::Deserialize;
use std::{cell::RefCell, rc::Rc, process::Command, sync::mpsc, time::Duration};

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Notice { id: u32, app: String, summary: String, body: String, #[serde(default)] progress: Option<u8>, #[serde(default)] missed: bool }
#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Snapshot { schema: u32, instance: String, dnd: bool, #[serde(default)] manual_dnd: Option<bool>, #[serde(default)] pause_remaining_seconds: Option<u32>, active: Vec<Notice>, history: Vec<Notice> }
enum Request { Dismiss(String, u32), Clear(String), Dnd(String, bool), Pause(String,String), Refresh }
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

fn format_notices<'a>(notices: impl IntoIterator<Item=&'a Notice>) -> String {
    let rendered=notices.into_iter().map(|notice| {
        let progress = notice.progress.filter(|v| *v <= 100).map(|v| format!(" · {v}%")).unwrap_or_default();
        let missed=if notice.missed{" · Missed"}else{""};
        format!("{} · {}{progress}{missed}\n{}\n{}", notice.id, notice.app, notice.summary, notice.body)
    })
        .collect::<Vec<_>>().join("\n\n");
    if rendered.is_empty(){"None".into()}else{rendered}
}

fn format_section(notices:&[Notice],missed_only:bool,grouped:bool)->(usize,String){
    let filtered:Vec<_>=notices.iter().filter(|notice|!missed_only||notice.missed).collect();
    let count=filtered.len();
    if !grouped || count==0{return (count,format_notices(filtered));}
    let mut apps=std::collections::BTreeMap::<&str,Vec<&Notice>>::new();
    for notice in filtered{apps.entry(&notice.app).or_default().push(notice);}
    let text=apps.into_iter().map(|(app,notices)|{
        let name=if app.is_empty(){"Unknown application"}else{app};
        format!("{name} ({})\n{}",notices.len(),format_notices(notices))
    }).collect::<Vec<_>>().join("\n\n");
    (count,text)
}

fn show_history(text:&gtk::TextView,snapshot:&Snapshot,missed_only:bool,grouped:bool){
    let (active,active_text)=format_section(&snapshot.active,missed_only,grouped);
    let (history,history_text)=format_section(&snapshot.history,missed_only,grouped);
    let prefix=if missed_only{"MISSED "}else{""};
    text.buffer().set_text(&format!("{prefix}ACTIVE ({active})\n{active_text}\n\n{prefix}CLOSED HISTORY ({history})\n{history_text}"));
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
    let pause_row=gtk::Box::new(gtk::Orientation::Horizontal,8);
    let pause_choice=gtk::ComboBoxText::new();
    for (id,label) in [("15m","15 minutes"),("30m","30 minutes"),("1h","1 hour"),("2h","2 hours"),("8h","8 hours")]{pause_choice.append(Some(id),label);}
    pause_choice.set_active_id(Some("30m"));
    let pause=gtk::Button::with_label("Pause popups");pause.set_sensitive(false);
    let resume=gtk::Button::with_label("End pause");resume.set_sensitive(false);
    let pause_status=gtk::Label::new(None);pause_status.set_xalign(0.0);
    pause_row.append(&pause_choice);pause_row.append(&pause);pause_row.append(&resume);root.append(&pause_row);root.append(&pause_status);
    let missed_only=gtk::CheckButton::with_label("Show missed notifications only");root.append(&missed_only);
    let grouped=gtk::CheckButton::with_label("Group notifications by application");root.append(&grouped);
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
                Ok(Request::Pause(instance,duration)) => call(&["pause",&instance,&duration]),
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
            if sender.try_send(Request::Dnd(snapshot.instance.clone(), !snapshot.manual_dnd.unwrap_or(snapshot.dnd))).is_err() { message.set_text("Notification service busy."); }
        }
    });
    for (button,cancel) in [(&pause,false),(&resume,true)]{
        let current=state.clone();let sender=requests.clone();let choice=pause_choice.clone();let message=status.clone();
        button.connect_clicked(move |_|{
            let Some(duration)= (if cancel {Some("off".to_owned())}else{choice.active_id().map(|v|v.to_string())}) else{return;};
            if let Some(snapshot)=current.borrow().as_ref(){
                if sender.try_send(Request::Pause(snapshot.instance.clone(),duration)).is_err(){message.set_text("Notification service busy.");}
            }
        });
    }
    let sender = requests.clone();
    refresh.connect_clicked(move |_| { let _ = sender.try_send(Request::Refresh); });

    let weak = window.downgrade();
    let mut last_filter=(false,false);
    glib::timeout_add_local(Duration::from_millis(150), move || {
        if weak.upgrade().is_none() { return glib::ControlFlow::Break; }
        let _ = &requests;
        let filter=(missed_only.is_active(),grouped.is_active());
        if filter!=last_filter{
            if let Some(snapshot)=state.borrow().as_ref(){show_history(&text,snapshot,filter.0,filter.1);}
            last_filter=filter;
        }
        while let Ok(reply) = replies.try_recv() {
            match reply.snapshot {
                Ok(snapshot) => {
                    let content_changed=state.borrow().as_ref().is_none_or(|old|old.instance!=snapshot.instance || old.active!=snapshot.active || old.history!=snapshot.history);
                    if content_changed {
                        let old_id = if state.borrow().as_ref().is_some_and(|old| old.instance == snapshot.instance) { choice.active_id() } else { None };
                        choice.remove_all();
                        for notice in &snapshot.active {
                            let label: String = notice.summary.chars().take(80).collect();
                            choice.append(Some(&notice.id.to_string()), &format!("{} · {label}", notice.id));
                        }
                        if !old_id.as_ref().is_some_and(|id| choice.set_active_id(Some(id.as_str()))) && !snapshot.active.is_empty() { choice.set_active(Some(0)); }
                        show_history(&text,&snapshot,filter.0,filter.1);
                        dismiss.set_sensitive(!snapshot.active.is_empty()); clear.set_sensitive(!snapshot.history.is_empty());
                    }
                    if state.borrow().as_ref()!=Some(&snapshot){
                        dnd.set_sensitive(true); dnd.set_label(if snapshot.manual_dnd.unwrap_or(snapshot.dnd) { "Do Not Disturb: On" } else { "Do Not Disturb: Off" });
                        pause.set_sensitive(snapshot.pause_remaining_seconds.is_some());
                        let remaining=snapshot.pause_remaining_seconds.unwrap_or(0);
                        resume.set_sensitive(remaining>0);
                        pause_status.set_text(&if remaining>0{format!("Temporary pause: {}m {:02}s remaining",remaining/60,remaining%60)}else{"No temporary pause".into()});
                        *state.borrow_mut() = Some(snapshot);
                    }
                    status.set_text(reply.action_error.as_deref().unwrap_or("Atelier inbox · content stays in session memory"));
                }
                Err(error) => {
                    *state.borrow_mut() = None;
                    choice.remove_all(); dismiss.set_sensitive(false); clear.set_sensitive(false); dnd.set_sensitive(false);pause.set_sensitive(false);resume.set_sensitive(false);pause_status.set_text("");
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
    fn grouping_retains_individual_entries_and_missed_filter(){
        let notice=|id,app:&str,missed|Notice{id,app:app.into(),summary:format!("Notice {id}"),body:String::new(),progress:None,missed};
        let notices=vec![notice(1,"B",false),notice(2,"A",true),notice(3,"A",false)];
        let (count,text)=format_section(&notices,false,true);
        assert_eq!(count,3);assert!(text.contains("A (2)"));assert!(text.contains("B (1)"));
        let (count,text)=format_section(&notices,true,true);
        assert_eq!(count,1);assert!(text.contains("Notice 2"));assert!(!text.contains("Notice 3"));
    }
    #[test]
    fn body_is_plain_text_and_history_empty_is_explicit() {
        let notice = Notice { id: 1, app: "Test".into(), summary: "<b>title</b>".into(), body: "text".into(), progress: Some(72), missed:true };
        let rendered = format_notices(&[notice]);
        assert!(rendered.contains("<b>title</b>"));
        assert!(rendered.contains("72%"));assert!(rendered.contains("Missed"));
        assert_eq!(format_notices(&[]), "None");
    }
}
