//! Network page backed by the independent NetworkManager provider.
use atelier_services::network::{network_action, network_snapshot, wifi_snapshot, NetworkAction, NetworkSnapshot, WifiSnapshot};
use gtk::{glib, prelude::*};
use std::{cell::Cell, rc::Rc, sync::mpsc, time::Duration};

enum Request { Refresh, Rescan, Action(NetworkAction) }
struct Reply {
    network: Result<NetworkSnapshot, String>,
    wifi: Option<Result<WifiSnapshot, String>>,
    action_error: Option<String>,
}

struct Form {
    root: gtk::Box,
    status: gtk::Label,
    devices: gtk::Label,
    nearby: gtk::Label,
    profiles: gtk::ComboBoxText,
    profile_actions: gtk::Box,
    wifi_actions: gtk::Box,
    rescan: gtk::Button,
    busy: Cell<bool>,
    requests: mpsc::SyncSender<Request>,
}
impl Form {
    fn request(&self, request: Request) {
        if self.busy.get() { return; }
        match self.requests.try_send(request) {
            Ok(()) => { self.busy.set(true); self.root.set_sensitive(false); self.status.set_text("Updating network status…"); }
            Err(_) => self.status.set_text("Network worker unavailable."),
        }
    }
    fn render(&self, reply: Reply) {
        let previous = self.profiles.active_id().map(|id| id.to_string());
        self.profiles.remove_all();
        let message = match reply.network {
            Ok(snapshot) => {
                let mut rows = vec![format!("Wi-Fi: {}", match snapshot.wifi_enabled { Some(true) => "Enabled", Some(false) => "Disabled", None => "Unknown" })];
                for device in &snapshot.devices {
                    rows.push(format!("{} · {} · {} · {}", device.name, device.kind, device.state, device.connection.as_deref().unwrap_or("No connection")));
                }
                if snapshot.devices.is_empty() { rows.push("No devices reported.".into()); }
                self.devices.set_text(&rows.join("\n"));
                for profile in &snapshot.profiles {
                    self.profiles.append(Some(&profile.uuid), &format!("{} ({})", profile.name, profile.kind));
                }
                if !previous.as_ref().is_some_and(|id| self.profiles.set_active_id(Some(id))) && !snapshot.profiles.is_empty() {
                    self.profiles.set_active(Some(0));
                }
                self.profile_actions.set_sensitive(!snapshot.profiles.is_empty());
                self.wifi_actions.set_sensitive(snapshot.wifi_enabled.is_some());
                self.rescan.set_sensitive(snapshot.wifi_enabled == Some(true));
                reply.action_error.unwrap_or_else(|| "Network status updated.".into())
            }
            Err(error) => {
                self.devices.set_text("Network information unavailable.");
                self.profile_actions.set_sensitive(false); self.wifi_actions.set_sensitive(false); self.rescan.set_sensitive(false);
                format!("Network unavailable: {error}{}", reply.action_error.map(|e| format!("; action: {e}")).unwrap_or_default())
            }
        };
        match reply.wifi {
            Some(Ok(wifi)) => {
                let rows: Vec<_> = wifi.access_points.iter().map(|ap| {
                    format!("{}{} · {} · {} · {}", if ap.active { "Connected: " } else { "" },
                        ap.ssid.as_deref().unwrap_or("Hidden network"), ap.bssid,
                        ap.signal.map(|value| format!("{value}%")).unwrap_or_else(|| "Unknown signal".into()),
                        ap.security.as_deref().unwrap_or("No security reported"))
                }).collect();
                self.nearby.set_text(&if rows.is_empty() { "No access points reported.".into() } else { rows.join("\n") });
            }
            Some(Err(error)) => self.nearby.set_text(&format!("Wi-Fi inventory unavailable: {error}")),
            None => self.nearby.set_text("Wi-Fi inventory unavailable or radio disabled."),
        }
        self.root.set_sensitive(true); self.busy.set(false); self.status.set_text(&message);
    }
}

pub fn build(page: &gtk::Box) {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let devices = crate::description("Refresh to discover network devices.");
    devices.set_selectable(true); root.append(&devices);
    let wifi_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let enable = gtk::Button::with_label("Enable Wi-Fi");
    let disable = gtk::Button::with_label("Disable Wi-Fi");
    wifi_actions.append(&enable); wifi_actions.append(&disable); wifi_actions.set_sensitive(false); root.append(&wifi_actions);
    root.append(&crate::description("Saved connections"));
    let profiles = gtk::ComboBoxText::new(); root.append(&profiles);
    let profile_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let activate = gtk::Button::with_label("Connect"); let deactivate = gtk::Button::with_label("Disconnect");
    profile_actions.append(&activate); profile_actions.append(&deactivate); profile_actions.set_sensitive(false); root.append(&profile_actions);
    root.append(&crate::description("Network actions apply immediately and may interrupt connectivity. New protected networks must be configured using your network manager."));
    let refresh = gtk::Button::with_label("Refresh status");
    let rescan = gtk::Button::with_label("Rescan Wi-Fi");
    rescan.set_sensitive(false);
    let refresh_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    refresh_row.append(&refresh); refresh_row.append(&rescan); root.append(&refresh_row);
    root.append(&crate::description("Nearby Wi-Fi (cached unless explicitly rescanned)"));
    let nearby = crate::description("Not yet loaded."); nearby.set_selectable(true); root.append(&nearby);
    let status = crate::description(""); page.append(&root); page.append(&status);

    let (requests, incoming) = mpsc::sync_channel::<Request>(1);
    let (outgoing, replies) = mpsc::sync_channel::<Reply>(1);
    std::thread::spawn(move || {
        while let Ok(request) = incoming.recv() {
            let (rescan, action_error) = match request {
                Request::Refresh => (false, None),
                Request::Rescan => (true, None),
                Request::Action(action) => (false, network_action(action).err().map(|e| e.to_string())),
            };
            let network = network_snapshot().map_err(|e| e.to_string());
            let wifi = if network.as_ref().is_ok_and(|state| state.wifi_enabled == Some(true) && state.devices.iter().any(|device| device.kind == "wifi")) {
                Some(wifi_snapshot(rescan).map_err(|e| e.to_string()))
            } else { None };
            if outgoing.send(Reply { network, wifi, action_error }).is_err() { break; }
        }
    });
    let form = Rc::new(Form { root, status, devices, nearby, profiles, profile_actions, wifi_actions, rescan: rescan.clone(), busy: Cell::new(false), requests });
    for (button, enabled) in [(&enable, true), (&disable, false)] {
        let weak = Rc::downgrade(&form);
        button.connect_clicked(move |_| { if let Some(form) = weak.upgrade() { form.request(Request::Action(NetworkAction::Wifi(enabled))); } });
    }
    for (button, connect) in [(&activate, true), (&deactivate, false)] {
        let weak = Rc::downgrade(&form);
        button.connect_clicked(move |_| {
            if let Some(form) = weak.upgrade() {
                if let Some(id) = form.profiles.active_id() {
                    let action = if connect { NetworkAction::Activate(id.to_string()) } else { NetworkAction::Deactivate(id.to_string()) };
                    form.request(Request::Action(action));
                }
            }
        });
    }
    let weak = Rc::downgrade(&form);
    refresh.connect_clicked(move |_| { if let Some(form) = weak.upgrade() { form.request(Request::Refresh); } });
    let weak = Rc::downgrade(&form);
    rescan.connect_clicked(move |_| { if let Some(form) = weak.upgrade() { form.request(Request::Rescan); } });
    let weak = Rc::downgrade(&form);
    page.connect_map(move |_| { if let Some(form) = weak.upgrade() { form.request(Request::Refresh); } });
    let page_weak = page.downgrade();
    glib::timeout_add_local(Duration::from_millis(150), move || {
        if page_weak.upgrade().is_none() { return glib::ControlFlow::Break; }
        match replies.try_recv() {
            Ok(reply) => form.render(reply),
            Err(mpsc::TryRecvError::Disconnected) => {
                form.root.set_sensitive(false); form.status.set_text("Network worker stopped.");
                return glib::ControlFlow::Break;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        glib::ControlFlow::Continue
    });
}
