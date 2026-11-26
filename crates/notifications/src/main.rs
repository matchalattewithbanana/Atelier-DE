//! Explicitly started notification service for an isolated or user-selected session.
mod store;
use anyhow::{bail, Context, Result};
use std::{collections::HashMap, sync::{Arc, Mutex, MutexGuard}, time::{Duration, Instant}};
use store::{Details, Store};
use zbus::{blocking::{connection::Builder, Connection, Proxy}, fdo, interface, object_server::SignalEmitter, zvariant::OwnedValue};

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const HISTORY_PATH: &str = "/org/atelier/Notifications";
const HISTORY_INTERFACE: &str = "org.atelier.Notifications1";
type Shared = Arc<Mutex<Store>>;

fn lock(shared: &Shared) -> fdo::Result<MutexGuard<'_, Store>> {
    shared.lock().map_err(|_| fdo::Error::Failed("Notification state unavailable".into()))
}

struct Notifications { state: Shared }

#[interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    fn get_capabilities(&self) -> Vec<String> { vec!["body".into(), "persistence".into()] }

    #[zbus(out_args("name", "vendor", "version", "spec_version"))]
    fn get_server_information(&self) -> (String, String, String, String) {
        ("Atelier Notifications".into(), "Atelier".into(), env!("CARGO_PKG_VERSION").into(), "1.2".into())
    }

    fn notify(&self, app_name: String, replaces_id: u32, app_icon: String, summary: String, body: String,
              actions: Vec<String>, hints: HashMap<String, OwnedValue>, expire_timeout: i32,
              #[zbus(header)] header: zbus::message::Header<'_>) -> fdo::Result<u32> {
        let transient = hints.get("transient").and_then(|value| bool::try_from(value).ok()).unwrap_or(false);
        let progress = hints.get("value").and_then(|value| i32::try_from(value).ok());
        let owner = header.sender().map(|name| name.as_str().to_owned());
        let details = Details::parse(actions, progress, app_icon, owner);
        lock(&self.state)?.notify_with(replaces_id, app_name, summary, body, transient, expire_timeout, Instant::now(), details)
            .map_err(|error| fdo::Error::LimitsExceeded(error.into()))
    }

    async fn close_notification(&self, id: u32, #[zbus(header)] header: zbus::message::Header<'_>, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> fdo::Result<()> {
        let closed = lock(&self.state)?.close_from(id, header.sender().map(|name| name.as_str()));
        if !closed { return Err(fdo::Error::Failed("Unknown notification ID".into())); }
        Self::notification_closed(&emitter, id, 3).await.map_err(|error| fdo::Error::Failed(error.to_string()))?;
        Ok(())
    }

    #[zbus(signal)]
    async fn action_invoked(emitter: &SignalEmitter<'_>, id: u32, action_key: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn notification_closed(emitter: &SignalEmitter<'_>, id: u32, reason: u32) -> zbus::Result<()>;
}

struct History { state: Shared }
#[interface(name = "org.atelier.Notifications1")]
impl History {
    fn list(&self) -> fdo::Result<String> {
        serde_json::to_string(&lock(&self.state)?.snapshot()).map_err(|error| fdo::Error::Failed(error.to_string()))
    }
    fn popup(&self) -> fdo::Result<String> {
        serde_json::to_string(&lock(&self.state)?.popup_snapshot()).map_err(|error| fdo::Error::Failed(error.to_string()))
    }
    fn set_dnd(&self, instance: String, enabled: bool) -> fdo::Result<()> {
        let mut state = lock(&self.state)?;
        if !state.matches_instance(&instance) { return Err(fdo::Error::Failed("Notification daemon changed".into())); }
        state.set_dnd(enabled); Ok(())
    }
    async fn dismiss(&self, instance: String, id: u32, #[zbus(connection)] connection: &zbus::Connection) -> fdo::Result<()> {
        {
            let mut state = lock(&self.state)?;
            if !state.matches_instance(&instance) || !state.close(id, 2) {
                return Err(fdo::Error::Failed("Notification changed or no longer exists".into()));
            }
        }
        connection.emit_signal(None::<&str>, PATH, NAME, "NotificationClosed", &(id, 2u32)).await
            .map_err(|error| fdo::Error::Failed(error.to_string()))?;
        Ok(())
    }
    async fn invoke(&self, instance: String, id: u32, revision: u64, key: String,
                    #[zbus(connection)] connection: &zbus::Connection) -> fdo::Result<()> {
        let owner = lock(&self.state)?.invoke(&instance, id, revision, &key, Instant::now())
            .ok_or_else(|| fdo::Error::Failed("Notification action changed or is unavailable".into()))?;
        // Send the opaque action key only to the originating bus connection.
        // Never execute notification-provided commands in the daemon.
        connection.emit_signal(Some(owner.as_str()), PATH, NAME, "ActionInvoked", &(id, key)).await
            .map_err(|error| fdo::Error::Failed(error.to_string()))?;
        Ok(())
    }
    fn clear_history_for(&self, instance: String) -> fdo::Result<()> {
        let mut state = lock(&self.state)?;
        if !state.matches_instance(&instance) { return Err(fdo::Error::Failed("Notification daemon changed".into())); }
        state.clear_history(); Ok(())
    }
    fn clear_history(&self) -> fdo::Result<()> {
        lock(&self.state)?.clear_history(); Ok(())
    }
}

fn daemon() -> Result<()> {
    let state = Arc::new(Mutex::new(Store::default()));
    let connection = Builder::session()?
        .serve_at(PATH, Notifications { state: Arc::clone(&state) })?
        .serve_at(HISTORY_PATH, History { state: Arc::clone(&state) })?
        .name(NAME)?
        .replace_existing_names(false)
        .allow_name_replacements(false)
        .build().context("acquire notification name without replacing existing daemon")?;
    println!("Atelier notification inbox ready (no graphical popups yet)");
    loop {
        std::thread::sleep(Duration::from_millis(200));
        let expired = state.lock().map_err(|_| anyhow::anyhow!("notification state unavailable"))?.expire(Instant::now());
        for id in expired {
            connection.emit_signal(None::<&str>, PATH, NAME, "NotificationClosed", &(id, 1u32))?;
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [mode] if mode == "daemon" => daemon(),
        [mode] if mode == "list" || mode == "popup" || mode == "clear-history" => {
            let connection = Connection::session()?;
            let proxy = Proxy::new(&connection, NAME, HISTORY_PATH, HISTORY_INTERFACE)?;
            if mode == "list" { let snapshot: String = proxy.call("List", &())?; println!("{snapshot}"); }
            else if mode == "popup" { let snapshot: String = proxy.call("Popup", &())?; println!("{snapshot}"); }
            else { let _: () = proxy.call("ClearHistory", &())?; }
            Ok(())
        }
        [mode, instance, value] if mode == "dnd" => {
            let enabled = match value.as_str() { "on" => true, "off" => false, _ => bail!("expected on or off") };
            let connection = Connection::session()?;
            let proxy = Proxy::new(&connection, NAME, HISTORY_PATH, HISTORY_INTERFACE)?;
            let _: () = proxy.call("SetDnd", &(instance.as_str(), enabled))?;
            Ok(())
        }
        [mode, instance, id] if mode == "dismiss" => {
            let id: u32 = id.parse().context("invalid notification ID")?;
            let connection = Connection::session()?;
            let proxy = Proxy::new(&connection, NAME, HISTORY_PATH, HISTORY_INTERFACE)?;
            let _: () = proxy.call("Dismiss", &(instance.as_str(), id))?;
            Ok(())
        }
        [mode, instance, id, revision, key] if mode == "invoke" => {
            let id: u32 = id.parse().context("invalid notification ID")?;
            let revision: u64 = revision.parse().context("invalid notification revision")?;
            let connection = Connection::session()?;
            let proxy = Proxy::new(&connection, NAME, HISTORY_PATH, HISTORY_INTERFACE)?;
            let _: () = proxy.call("Invoke", &(instance.as_str(), id, revision, key.as_str()))?;
            Ok(())
        }
        [mode, instance] if mode == "clear-history" => {
            let connection = Connection::session()?;
            let proxy = Proxy::new(&connection, NAME, HISTORY_PATH, HISTORY_INTERFACE)?;
            let _: () = proxy.call("ClearHistoryFor", &(instance.as_str(),))?;
            Ok(())
        }
        [mode] if mode == "--help" || mode == "-h" => {
            println!("atelier-notifications daemon | list | popup | clear-history [INSTANCE] | dismiss INSTANCE ID | invoke INSTANCE ID REVISION KEY | dnd INSTANCE on|off\nExperimental, explicit opt-in notification inbox. Does not replace another daemon.");
            Ok(())
        }
        _ => bail!("usage: atelier-notifications --help"),
    }
}
