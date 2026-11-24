//! Independent audio controls. No provider calls on the GTK thread.
use atelier_services::audio::{audio_action, audio_snapshot, AudioAction, AudioSnapshot, Device, Direction};
use gtk::{glib, prelude::*};
use std::{cell::{Cell, RefCell}, rc::Rc, sync::mpsc, time::Duration};

enum Request { Refresh, Apply(Direction, String, AudioAction) }
struct Reply { action_error: Option<String>, snapshot: Result<AudioSnapshot, String> }

struct Form {
    root: gtk::Box,
    direction: gtk::ComboBoxText,
    device: gtk::ComboBoxText,
    details: gtk::Label,
    volume: gtk::SpinButton,
    actions: gtk::Box,
    status: gtk::Label,
    snapshot: RefCell<Option<AudioSnapshot>>,
    busy: Cell<bool>,
    requests: mpsc::SyncSender<Request>,
}

impl Form {
    fn direction(&self) -> Direction {
        if self.direction.active_id().as_deref() == Some("input") { Direction::Input } else { Direction::Output }
    }
    fn devices<'a>(&self, snapshot: &'a AudioSnapshot) -> (&'a [Device], Option<&'a str>) {
        match self.direction() {
            Direction::Output => (&snapshot.outputs, snapshot.default_output.as_deref()),
            Direction::Input => (&snapshot.inputs, snapshot.default_input.as_deref()),
        }
    }
    fn request(&self, request: Request) {
        if self.busy.get() { return; }
        match self.requests.try_send(request) {
            Ok(()) => {
                self.busy.set(true);
                self.root.set_sensitive(false);
                self.status.set_text("Updating audio…");
            }
            Err(_) => self.status.set_text("Audio worker unavailable."),
        }
    }
    fn apply(&self, action: AudioAction) {
        if let Some(id) = self.device.active_id() { self.request(Request::Apply(self.direction(), id.to_string(), action)); }
    }
    fn populate(&self, preserve_selection: bool) {
        let previous = if preserve_selection { self.device.active_id().map(|id| id.to_string()) } else { None };
        let (entries, default) = {
            let snapshot = self.snapshot.borrow();
            if let Some(snapshot) = snapshot.as_ref() {
                let (devices, default) = self.devices(snapshot);
                let entries: Vec<_> = devices.iter().map(|device| {
                    let suffix = if device.monitor { " (monitor)" } else { "" };
                    (device.id.clone(), format!("{}{suffix}", device.description))
                }).collect();
                (entries, default.map(str::to_owned))
            } else { (Vec::new(), None) }
        };
        self.device.remove_all();
        for (id, title) in &entries { self.device.append(Some(id), title); }
        let selected = previous.filter(|id| entries.iter().any(|(entry, _)| entry == id)).or(default);
        if !selected.as_ref().is_some_and(|id| self.device.set_active_id(Some(id))) && !entries.is_empty() {
            self.device.set_active(Some(0));
        }
        self.update_selected();
    }
    fn update_selected(&self) {
        let snapshot = self.snapshot.borrow();
        let id = self.device.active_id();
        let selected = snapshot.as_ref().and_then(|snapshot| {
            let (devices, default) = self.devices(snapshot);
            let id = id.as_deref()?;
            devices.iter().find(|d| d.id == id).map(|d| (d, default == Some(id)))
        });
        self.actions.set_sensitive(selected.is_some());
        if let Some((device, is_default)) = selected {
            let channels: Vec<_> = device.channels.iter().map(|channel| {
                format!("{}: {}", channel.name, channel.percent.map(|p| format!("{p}%")).unwrap_or_else(|| "unknown".into()))
            }).collect();
            let mute = match device.muted { Some(true) => "Muted", Some(false) => "Unmuted", None => "Mute status unknown" };
            self.details.set_text(&format!("{} · {}\n{}",
                if is_default { "Default device" } else { "Not the default" }, mute,
                if channels.is_empty() { "Volume unavailable".into() } else { channels.join(" · ") }));
            let level = device.channels.iter().filter_map(|channel| channel.percent).max().unwrap_or(0);
            self.volume.set_value(f64::from(level.min(100)));
        } else {
            self.details.set_text("No device selected.");
            self.volume.set_value(0.0);
        }
    }
}

pub fn build(page: &gtk::Box) {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let direction = gtk::ComboBoxText::new();
    direction.append(Some("output"), "Playback output");
    direction.append(Some("input"), "Recording input");
    direction.set_active_id(Some("output"));
    root.append(&direction);
    let device = gtk::ComboBoxText::new();
    root.append(&device);
    let details = crate::description("Refresh to discover audio devices.");
    root.append(&details);
    let actions = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let volume = gtk::SpinButton::with_range(0.0, 100.0, 1.0);
    volume.set_hexpand(true);
    let volume_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let set_volume = gtk::Button::with_label("Apply volume (%)");
    volume_row.append(&volume); volume_row.append(&set_volume); actions.append(&volume_row);
    actions.append(&crate::description("Applying volume changes all channels. Amplified levels above 100% are displayed above, not applied automatically."));
    let mute_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let mute = gtk::Button::with_label("Mute");
    let unmute = gtk::Button::with_label("Unmute");
    let default_device = gtk::Button::with_label("Set as default");
    for button in [&mute, &unmute, &default_device] { button.set_hexpand(true); mute_row.append(button); }
    actions.append(&mute_row); actions.set_sensitive(false); root.append(&actions);
    let refresh = gtk::Button::with_label("Refresh devices"); root.append(&refresh);
    let status = crate::description("Audio actions apply immediately; other preferences still use Save.");
    page.append(&root); page.append(&status);

    let (requests, incoming) = mpsc::sync_channel::<Request>(1);
    let (outgoing, replies) = mpsc::sync_channel::<Reply>(1);
    std::thread::spawn(move || {
        while let Ok(request) = incoming.recv() {
            let action_error = match request {
                Request::Refresh => None,
                Request::Apply(direction, id, action) => audio_action(direction, &id, action).err().map(|err| err.to_string()),
            };
            let snapshot = audio_snapshot().map_err(|err| err.to_string());
            if outgoing.send(Reply { action_error, snapshot }).is_err() { break; }
        }
    });

    let form = Rc::new(Form { root, direction, device, details, volume, actions, status, snapshot: RefCell::new(None), busy: Cell::new(false), requests });
    let weak = Rc::downgrade(&form);
    form.direction.connect_changed(move |_| { if let Some(form) = weak.upgrade() { form.populate(false); } });
    let weak = Rc::downgrade(&form);
    form.device.connect_changed(move |_| { if let Some(form) = weak.upgrade() { form.update_selected(); } });
    let weak = Rc::downgrade(&form);
    refresh.connect_clicked(move |_| { if let Some(form) = weak.upgrade() { form.request(Request::Refresh); } });
    let weak = Rc::downgrade(&form);
    set_volume.connect_clicked(move |_| { if let Some(form) = weak.upgrade() { form.apply(AudioAction::Volume(form.volume.value_as_int() as u8)); } });
    for (button, action) in [(&mute, AudioAction::Mute(true)), (&unmute, AudioAction::Mute(false)), (&default_device, AudioAction::Default)] {
        let weak = Rc::downgrade(&form);
        button.connect_clicked(move |_| { if let Some(form) = weak.upgrade() { form.apply(action); } });
    }
    let weak = Rc::downgrade(&form);
    page.connect_map(move |_| { if let Some(form) = weak.upgrade() { form.request(Request::Refresh); } });
    let page_weak = page.downgrade();
    glib::timeout_add_local(Duration::from_millis(150), move || {
        if page_weak.upgrade().is_none() { return glib::ControlFlow::Break; }
        match replies.try_recv() {
            Ok(reply) => {
                let message = match reply.snapshot {
                    Ok(snapshot) => {
                        *form.snapshot.borrow_mut() = Some(snapshot);
                        reply.action_error.unwrap_or_else(|| "Audio status updated. Changes apply immediately.".into())
                    }
                    Err(error) => {
                        *form.snapshot.borrow_mut() = None;
                        format!("Audio unavailable: {error}{}", reply.action_error.map(|e| format!("; action: {e}")).unwrap_or_default())
                    }
                };
                form.populate(true);
                form.busy.set(false);
                form.root.set_sensitive(true);
                form.status.set_text(&message);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                form.root.set_sensitive(false);
                form.status.set_text("Audio worker stopped.");
                return glib::ControlFlow::Break;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        glib::ControlFlow::Continue
    });
}
