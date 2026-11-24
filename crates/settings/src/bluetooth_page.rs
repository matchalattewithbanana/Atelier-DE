//! Independent paired-device management; all BlueZ calls stay off the GTK thread.
use atelier_services::bluetooth::{bluetooth_action, bluetooth_snapshot, BluetoothAction, BluetoothSnapshot};
use gtk::{glib, prelude::*};
use std::{cell::Cell, rc::Rc, sync::mpsc, time::Duration};

enum Request { Refresh, Action(BluetoothAction) }
struct Reply { snapshot: Result<BluetoothSnapshot, String>, action_error: Option<String> }

struct Form {
    root: gtk::Box,
    controllers: gtk::Label,
    devices: gtk::ComboBoxText,
    power_actions: gtk::Box,
    device_actions: gtk::Box,
    status: gtk::Label,
    busy: Cell<bool>,
    requests: mpsc::SyncSender<Request>,
}
impl Form {
    fn request(&self, request: Request) {
        if self.busy.get() { return; }
        match self.requests.try_send(request) {
            Ok(()) => { self.busy.set(true); self.root.set_sensitive(false); self.status.set_text("Updating Bluetooth…"); }
            Err(_) => self.status.set_text("Bluetooth worker unavailable."),
        }
    }
    fn render(&self, reply: Reply) {
        let previous = self.devices.active_id().map(|id| id.to_string());
        self.devices.remove_all();
        self.device_actions.set_sensitive(false);
        self.power_actions.set_sensitive(false);
        let message = match reply.snapshot {
            Ok(snapshot) => {
                let mut rows = vec![format!("Bluetooth power: {}", match snapshot.powered { Some(true) => "On", Some(false) => "Off", None => "Unknown" })];
                for controller in &snapshot.controllers {
                    rows.push(format!("{} · {}", controller.name, controller.address));
                }
                if snapshot.controllers.is_empty() { rows.push("No Bluetooth adapters detected.".into()); }
                if snapshot.controllers.len() > 1 { rows.push("Multiple adapters detected. Controls are unavailable until adapter selection is supported.".into()); }
                self.controllers.set_text(&rows.join("\n"));
                let single = snapshot.controllers.len() == 1;
                self.power_actions.set_sensitive(single);
                for device in &snapshot.paired_devices {
                    let state = match device.connected { Some(true) => "Connected", Some(false) => "Disconnected", None => "Unknown" };
                    self.devices.append(Some(&device.address), &format!("{} · {} · {}", device.name, device.address, state));
                }
                if !previous.as_ref().is_some_and(|id| self.devices.set_active_id(Some(id))) && !snapshot.paired_devices.is_empty() {
                    self.devices.set_active(Some(0));
                }
                self.device_actions.set_sensitive(single && snapshot.powered == Some(true) && !snapshot.paired_devices.is_empty());
                reply.action_error.unwrap_or_else(|| {
                    if single && snapshot.paired_devices.is_empty() { "No paired devices reported.".into() } else { "Bluetooth status updated.".into() }
                })
            }
            Err(error) => {
                self.controllers.set_text("Bluetooth information unavailable.");
                format!("Bluetooth unavailable: {error}{}", reply.action_error.map(|e| format!("; action: {e}")).unwrap_or_default())
            }
        };
        self.root.set_sensitive(true); self.busy.set(false); self.status.set_text(&message);
    }
}

pub fn build(page: &gtk::Box) {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let controllers = crate::description("Refresh to discover Bluetooth adapters.");
    controllers.set_selectable(true); root.append(&controllers);
    let power_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let on = gtk::Button::with_label("Turn on"); let off = gtk::Button::with_label("Turn off");
    power_actions.append(&on); power_actions.append(&off); power_actions.set_sensitive(false); root.append(&power_actions);
    root.append(&crate::description("Paired devices"));
    let devices = gtk::ComboBoxText::new(); root.append(&devices);
    let device_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let connect = gtk::Button::with_label("Connect"); let disconnect = gtk::Button::with_label("Disconnect");
    device_actions.append(&connect); device_actions.append(&disconnect); device_actions.set_sensitive(false); root.append(&device_actions);
    root.append(&crate::description("Bluetooth actions apply immediately. Turning off the adapter may disconnect peripherals. Pairing and discovery of new devices are not supported here yet."));
    let refresh = gtk::Button::with_label("Refresh Bluetooth"); root.append(&refresh);
    let status = crate::description(""); page.append(&root); page.append(&status);

    let (requests, incoming) = mpsc::sync_channel::<Request>(1);
    let (outgoing, replies) = mpsc::sync_channel::<Reply>(1);
    std::thread::spawn(move || {
        while let Ok(request) = incoming.recv() {
            let action_error = match request {
                Request::Refresh => None,
                Request::Action(action) => bluetooth_action(action).err().map(|e| e.to_string()),
            };
            let snapshot = bluetooth_snapshot().map_err(|e| e.to_string());
            if outgoing.send(Reply { snapshot, action_error }).is_err() { break; }
        }
    });
    let form = Rc::new(Form { root, controllers, devices, power_actions, device_actions, status, busy: Cell::new(false), requests });
    for (button, enabled) in [(&on, true), (&off, false)] {
        let weak = Rc::downgrade(&form);
        button.connect_clicked(move |_| { if let Some(form) = weak.upgrade() { form.request(Request::Action(BluetoothAction::Power(enabled))); } });
    }
    for (button, connect) in [(&connect, true), (&disconnect, false)] {
        let weak = Rc::downgrade(&form);
        button.connect_clicked(move |_| {
            if let Some(form) = weak.upgrade() {
                if let Some(id) = form.devices.active_id() {
                    let action = if connect { BluetoothAction::Connect(id.to_string()) } else { BluetoothAction::Disconnect(id.to_string()) };
                    form.request(Request::Action(action));
                }
            }
        });
    }
    let weak = Rc::downgrade(&form);
    refresh.connect_clicked(move |_| { if let Some(form) = weak.upgrade() { form.request(Request::Refresh); } });
    let weak = Rc::downgrade(&form);
    page.connect_map(move |_| { if let Some(form) = weak.upgrade() { form.request(Request::Refresh); } });
    let page_weak = page.downgrade();
    glib::timeout_add_local(Duration::from_millis(150), move || {
        if page_weak.upgrade().is_none() { return glib::ControlFlow::Break; }
        match replies.try_recv() {
            Ok(reply) => form.render(reply),
            Err(mpsc::TryRecvError::Disconnected) => {
                form.root.set_sensitive(false); form.status.set_text("Bluetooth worker stopped.");
                return glib::ControlFlow::Break;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        glib::ControlFlow::Continue
    });
}
