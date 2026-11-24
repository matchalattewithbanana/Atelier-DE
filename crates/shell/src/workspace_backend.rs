//! Product-neutral workspace adapter. Reuse Mango Atelier's normalized CLI
//! rather than duplicating compositor-specific commands in the GTK shell.
use atelier_config::workspace_paths;
use std::{
    env, fs, io::Read,
    os::unix::{fs::FileTypeExt, net::UnixDatagram},
    path::PathBuf, process::Command,
    sync::{mpsc::{self, SyncSender}, Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot { pub active: [bool; 9] }

fn parse_snapshot(expected: &str, bytes: &[u8]) -> Option<Snapshot> {
    let data: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if data.get("compositor")?.as_str()? != expected { return None; }
    let monitors = data.get("monitors")?.as_array()?;
    if monitors.is_empty() { return None; }
    let mut active = [false; 9];
    for monitor in monitors {
        for tag in monitor.get("tags")?.as_array()? {
            let index = tag.get("index")?.as_u64()?;
            if (1..=9).contains(&index) && tag.get("is_active")?.as_bool()? {
                active[(index - 1) as usize] = true;
            }
        }
    }
    Some(Snapshot { active })
}

fn external_name(marked: &[&'static str], assigned: Option<&str>, desktop: &str) -> Option<&'static str> {
    let supported = ["mango", "hyprland", "niri"];
    if marked.len() > 1 { return None; }
    if let Some(name) = marked.first() {
        if assigned.is_some_and(|value| value != *name) { return None; }
        return Some(*name);
    }
    if let Some(value) = assigned { return supported.into_iter().find(|name| *name == value); }
    let names: Vec<_> = supported.into_iter().filter(|name| {
        desktop.split(':').any(|part| part.eq_ignore_ascii_case(name) || (*name == "mango" && part.eq_ignore_ascii_case("mangowm")))
    }).collect();
    if names.len() == 1 { Some(names[0]) } else { None }
}

pub fn installed_helper(name: &str) -> Option<PathBuf> {
    // The existing Atelier installer places its CLI symlinks here. PATH is
    // supported for package-managed installs, without invoking a shell.
    let local = env::var_os("HOME").map(PathBuf::from).map(|p| p.join(".local/bin").join(name));
    if let Some(path) = local.filter(|p| p.is_file()) { return Some(path); }
    env::var_os("PATH").and_then(|path| env::split_paths(&path).map(|p| p.join(name)).find(|p| p.is_file()))
}

pub enum WorkspaceBackend {
    Native { command: PathBuf, state: PathBuf },
    External { name: &'static str, requests: SyncSender<u8>, snapshot: Arc<Mutex<Option<Snapshot>>> },
    Unavailable,
}

impl WorkspaceBackend {
    pub fn detect() -> Self {
        let Some(display) = env::var("WAYLAND_DISPLAY").ok().filter(|s| !s.is_empty()) else { return Self::Unavailable; };
        if let Ok((command, state)) = workspace_paths(&display) {
            if fs::metadata(&command).is_ok_and(|m| m.file_type().is_socket()) || display.starts_with("wayland-atelier-") {
                return Self::Native { command, state };
            }
        }
        let marked: Vec<_> = [("mango", "MANGO_INSTANCE_SIGNATURE"), ("hyprland", "HYPRLAND_INSTANCE_SIGNATURE"), ("niri", "NIRI_SOCKET")]
            .into_iter().filter(|(_, key)| env::var_os(key).is_some_and(|value| !value.is_empty())).map(|(name, _)| name).collect();
        let assigned = env::var("ATELIER_COMPOSITOR").ok();
        let desktop = format!("{}:{}", env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(), env::var("XDG_SESSION_DESKTOP").unwrap_or_default());
        let Some(name) = external_name(&marked, assigned.as_deref(), &desktop) else { return Self::Unavailable; };
        let Some(helper) = installed_helper("atelier-workspace") else { return Self::Unavailable; };
        let (requests, receiver) = mpsc::sync_channel::<u8>(4);
        let snapshot = Arc::new(Mutex::new(None));
        let target = Arc::clone(&snapshot);
        // One worker, no overlapping queries and no blocking GTK main thread.
        std::thread::spawn(move || {
            loop {
                let result = Command::new(&helper).arg("status").env("ATELIER_COMPOSITOR", name).output().ok()
                    .filter(|result| result.status.success()).and_then(|result| parse_snapshot(name, &result.stdout));
                if let Ok(mut state) = target.lock() { *state = result; }
                match receiver.recv_timeout(Duration::from_millis(1500)) {
                    Ok(number) if (1..=9).contains(&number) => {
                        // Never infer a successful selection; refresh authoritative state next.
                        let _ = Command::new(&helper).args(["focus", &number.to_string()]).env("ATELIER_COMPOSITOR", name).output();
                    }
                    Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        Self::External { name, requests, snapshot }
    }

    pub fn count(&self) -> usize {
        match self { Self::Native { .. } => 4, Self::External { .. } => 9, Self::Unavailable => 0 }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Native { .. } => "Atelier DE",
            Self::External { name: "mango", .. } => "Mango Atelier",
            Self::External { name: "hyprland", .. } => "Hyprland",
            Self::External { name: "niri", .. } => "Niri",
            _ => "Standalone",
        }
    }

    pub fn is_external(&self) -> bool { matches!(self, Self::External { .. }) }

    pub fn snapshot(&self) -> Option<Snapshot> {
        match self {
            Self::Native { command, state } => {
                if !fs::metadata(command).is_ok_and(|m| m.file_type().is_socket()) { return None; }
                let mut byte = [0u8];
                fs::File::open(state).ok()?.read_exact(&mut byte).ok()?;
                if !(1..=4).contains(&byte[0]) { return None; }
                let mut active = [false; 9]; active[usize::from(byte[0] - 1)] = true;
                Some(Snapshot { active })
            }
            Self::External { snapshot, .. } => snapshot.lock().ok()?.clone(),
            Self::Unavailable => None,
        }
    }

    pub fn focus(&self, number: u8) {
        if number == 0 || usize::from(number) > self.count() || self.snapshot().is_none() { return; }
        match self {
            Self::Native { command, .. } => {
                if let Ok(socket) = UnixDatagram::unbound() { let _ = socket.send_to(&[number], command); }
            }
            Self::External { requests, .. } => { let _ = requests.try_send(number); }
            Self::Unavailable => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_ambiguous_or_unknown_sessions() {
        assert_eq!(external_name(&["mango", "niri"], None, ""), None);
        assert_eq!(external_name(&["niri"], Some("mango"), ""), None);
        assert_eq!(external_name(&[], None, "GNOME"), None);
        assert_eq!(external_name(&[], Some("mango"), ""), Some("mango"));
        assert_eq!(external_name(&["hyprland"], None, ""), Some("hyprland"));
        assert_eq!(external_name(&[], None, "niri"), Some("niri"));
    }

    #[test]
    fn status_is_backend_scoped_and_bounded() {
        let input = br#"{"compositor":"niri","monitors":[{"tags":[{"index":2,"is_active":true},{"index":99,"is_active":true}]}]}"#;
        let snapshot = parse_snapshot("niri", input).unwrap();
        assert!(snapshot.active[1]);
        assert_eq!(snapshot.active.iter().filter(|active| **active).count(), 1);
        assert!(parse_snapshot("mango", input).is_none());
        assert!(parse_snapshot("niri", b"{}").is_none());
    }
}
