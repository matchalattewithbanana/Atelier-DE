//! On-demand media controller; blocking MPRIS bridge calls run off the GTK thread.
use atelier_services::media::{media_action, media_snapshot, MediaAction, MediaSnapshot, Player};
use gtk::{glib, prelude::*};
use std::{cell::RefCell, rc::Rc, sync::mpsc, time::Duration};

enum Request { Action(String, MediaAction) }
enum Event { Snapshot(Result<MediaSnapshot, String>), ActionError(String) }

pub fn show(app: &gtk::Application) {
    if let Some(window) = app.windows().into_iter().find(|window| window.widget_name() == "atelier-media") {
        window.present(); return;
    }
    let window = gtk::ApplicationWindow::builder().application(app).title("Atelier · Media").default_width(440).default_height(300).build();
    window.set_widget_name("atelier-media");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(24); content.set_margin_bottom(24); content.set_margin_start(24); content.set_margin_end(24);
    let title = gtk::Label::new(Some("Now playing"));
    title.set_xalign(0.0); title.add_css_class("title-1"); content.append(&title);
    let choice = gtk::ComboBoxText::new(); content.append(&choice);
    let track = gtk::Label::new(Some("Looking for media players…"));
    track.set_xalign(0.0); track.set_wrap(true); track.add_css_class("title-2"); content.append(&track);
    let artist = gtk::Label::new(None); artist.set_xalign(0.0); artist.set_wrap(true); content.append(&artist);
    let playback = gtk::Label::new(None); playback.set_xalign(0.0); playback.add_css_class("dim-label"); content.append(&playback);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let previous = gtk::Button::with_label("Previous");
    let toggle = gtk::Button::with_label("Play / pause");
    let next = gtk::Button::with_label("Next");
    for button in [&previous, &toggle, &next] { button.set_hexpand(true); button.set_sensitive(false); controls.append(button); }
    content.append(&controls);
    let status = gtk::Label::new(None); status.set_xalign(0.0); status.set_wrap(true); status.add_css_class("dim-label"); content.append(&status);

    let (requests, receiver) = mpsc::sync_channel::<Request>(1);
    let (events, updates) = mpsc::sync_channel::<Event>(4);
    std::thread::spawn(move || loop {
        let snapshot = media_snapshot().map_err(|error| error.to_string());
        if events.send(Event::Snapshot(snapshot)).is_err() { break; }
        match receiver.recv_timeout(Duration::from_secs(3)) {
            Ok(Request::Action(id, action)) => {
                if let Err(error) = media_action(&id, action) {
                    if events.send(Event::ActionError(error.to_string())).is_err() { break; }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    });

    let players: Rc<RefCell<Vec<Player>>> = Rc::new(RefCell::new(Vec::new()));
    let display = {
        let players = Rc::clone(&players);
        let track = track.clone(); let artist = artist.clone(); let playback = playback.clone();
        let previous = previous.clone(); let toggle = toggle.clone(); let next = next.clone();
        move |choice: &gtk::ComboBoxText| {
            let players = players.borrow();
            let selected = choice.active_id();
            let selected = selected.as_ref().and_then(|id| players.iter().find(|player| player.id == id.as_str()));
            let available = selected.is_some_and(|player| player.status.is_some() && !players.iter().any(|other| other.id.starts_with(&format!("{}.", player.id))));
            for button in [&previous, &toggle, &next] { button.set_sensitive(available); }
            track.set_text(selected.and_then(|p| p.title.as_deref()).unwrap_or("No track information"));
            artist.set_text(selected.and_then(|p| p.artist.as_deref()).unwrap_or(""));
            playback.set_text(selected.and_then(|p| p.status.as_deref()).unwrap_or("Playback status unavailable"));
        }
    };
    let update_display = display.clone();
    choice.connect_changed(move |combo| update_display(combo));

    for (button, action) in [(&previous, MediaAction::Previous), (&toggle, MediaAction::Toggle), (&next, MediaAction::Next)] {
        let choice = choice.clone(); let sender = requests.clone(); let status = status.clone();
        button.connect_clicked(move |_| {
            if let Some(id) = choice.active_id() {
                status.set_text(if sender.try_send(Request::Action(id.to_string(), action)).is_ok() { "Playback request sent." } else { "Media service busy or unavailable." });
            }
        });
    }
    let weak = window.downgrade();
    glib::timeout_add_local(Duration::from_millis(150), move || {
        if weak.upgrade().is_none() { return glib::ControlFlow::Break; }
        // Keep the receiver and request sender alive only while this window exists.
        let _ = &requests;
        while let Ok(event) = updates.try_recv() {
            match event {
                Event::ActionError(error) => status.set_text(&error),
                Event::Snapshot(result) => {
                    let new_players = match result {
                        Ok(snapshot) => {
                            if snapshot.players.is_empty() { status.set_text("No MPRIS media players detected."); }
                            else if status.text().as_str() == "No MPRIS media players detected." || status.text().starts_with("Media unavailable:") { status.set_text(""); }
                            snapshot.players
                        }
                        Err(error) => { status.set_text(&format!("Media unavailable: {error}")); Vec::new() }
                    };
                    let changed = !players.borrow().iter().map(|p| &p.id).eq(new_players.iter().map(|p| &p.id));
                    let old_id = choice.active_id();
                    *players.borrow_mut() = new_players;
                    if changed {
                        choice.remove_all();
                        let ids: Vec<_> = players.borrow().iter().map(|player| player.id.clone()).collect();
                        for id in ids { choice.append(Some(&id), &id); }
                        if !old_id.as_ref().is_some_and(|id| choice.set_active_id(Some(id.as_str()))) { choice.set_active(Some(0)); }
                    }
                    display(&choice);
                }
            }
        }
        glib::ControlFlow::Continue
    });
    window.set_child(Some(&content)); window.present();
}
