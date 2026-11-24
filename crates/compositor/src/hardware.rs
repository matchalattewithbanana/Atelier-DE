//! Read-only Linux hardware inventory. No device opens, ioctls or session acquisition.

use std::{fs, path::{Path, PathBuf}};

#[derive(Debug, PartialEq, Eq)]
struct Node {
    name: String,
    device: PathBuf,
    present: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct Connector {
    name: String,
    status: Option<String>,
    modes: Vec<String>,
}

#[derive(Debug, Default)]
struct Inventory {
    cards: Vec<Node>,
    render_nodes: Vec<Node>,
    connectors: Vec<Connector>,
    input_nodes: Vec<Node>,
    warnings: Vec<String>,
}

fn numbered(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn names(path: &Path, warnings: &mut Vec<String>) -> Vec<String> {
    let mut result = Vec::new();
    match fs::read_dir(path) {
        Ok(entries) => for entry in entries {
            match entry {
                Ok(entry) => {
                    if let Ok(name) = entry.file_name().into_string() { result.push(name); }
                }
                Err(error) => warnings.push(format!("{}: {error}", path.display())),
            }
        },
        Err(error) => warnings.push(format!("{}: {error}", path.display())),
    }
    result.sort();
    result
}

fn node(name: String, directory: &Path) -> Node {
    let device = directory.join(&name);
    let present = device.exists();
    Node { name, device, present }
}

fn read_attribute(path: &Path, warnings: &mut Vec<String>) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(value) => Some(value.trim().to_owned()),
        Err(error) => {
            warnings.push(format!("{}: {error}", path.display()));
            None
        }
    }
}

impl Inventory {
    fn scan(sys: &Path, dev: &Path) -> Self {
        let mut inventory = Self::default();
        let drm = sys.join("class/drm");
        let drm_dev = dev.join("dri");
        for name in names(&drm, &mut inventory.warnings) {
            if numbered(&name, "card") {
                inventory.cards.push(node(name, &drm_dev));
            } else if numbered(&name, "renderD") {
                inventory.render_nodes.push(node(name, &drm_dev));
            } else if name.split_once('-').is_some_and(|(card, connector)| numbered(card, "card") && !connector.is_empty()) {
                let path = drm.join(&name);
                let status = read_attribute(&path.join("status"), &mut inventory.warnings);
                let modes = if status.as_deref() == Some("connected") {
                    read_attribute(&path.join("modes"), &mut inventory.warnings)
                        .map(|value| value.lines().filter(|line| !line.is_empty()).map(str::to_owned).collect())
                        .unwrap_or_default()
                } else { Vec::new() };
                inventory.connectors.push(Connector { name, status, modes });
            }
        }
        let input_dev = dev.join("input");
        for name in names(&sys.join("class/input"), &mut inventory.warnings) {
            if numbered(&name, "event") { inventory.input_nodes.push(node(name, &input_dev)); }
        }
        inventory
    }
}

pub fn report() {
    let inventory = Inventory::scan(Path::new("/sys"), Path::new("/dev"));
    println!("Atelier native hardware inventory (read-only)");
    println!("Session type: {}", std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into()));
    println!("Seat: {}", std::env::var("XDG_SEAT").unwrap_or_else(|_| "unspecified".into()));
    for (label, nodes) in [
        ("DRM primary nodes", &inventory.cards),
        ("DRM render nodes", &inventory.render_nodes),
        ("Input event nodes", &inventory.input_nodes),
    ] {
        println!("{label}: {}", nodes.len());
        for node in nodes {
            println!("  {}: {}", node.device.display(), if node.present { "present" } else { "not visible" });
        }
    }
    println!("Connectors: {}", inventory.connectors.len());
    for connector in &inventory.connectors {
        println!("  {}: {}", connector.name, connector.status.as_deref().unwrap_or("unknown"));
        for mode in &connector.modes { println!("    {mode}"); }
    }
    for warning in &inventory.warnings { eprintln!("Inventory warning: {warning}"); }
    println!("Inventory only; access permissions, seat ownership, EGL/GBM and KMS compatibility are not verified.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_only_expected_nodes() {
        for name in ["card0", "card12"] { assert!(numbered(name, "card")); }
        for name in ["card", "card0-HDMI-A-1", "card-1", "renderD128", "cardx"] { assert!(!numbered(name, "card")); }
    }

    #[test]
    fn scans_fixture_without_opening_devices() {
        let fixture = tempfile::tempdir().unwrap();
        let sys = fixture.path().join("sys");
        let dev = fixture.path().join("dev");
        for name in ["card0", "renderD128", "card0-DP-1", "card0-HDMI-A-1", "version"] {
            fs::create_dir_all(sys.join("class/drm").join(name)).unwrap();
        }
        let connected = sys.join("class/drm/card0-DP-1");
        fs::write(connected.join("status"), "connected\n").unwrap();
        fs::write(connected.join("modes"), "1920x1080\n1280x720\n").unwrap();
        fs::write(sys.join("class/drm/card0-HDMI-A-1/status"), "disconnected\n").unwrap();
        fs::create_dir_all(sys.join("class/input/event7")).unwrap();
        fs::create_dir_all(sys.join("class/input/mouse0")).unwrap();
        fs::create_dir_all(dev.join("dri")).unwrap();
        fs::write(dev.join("dri/card0"), "").unwrap();
        let found = Inventory::scan(&sys, &dev);
        assert_eq!(found.cards.len(), 1);
        assert!(found.cards[0].present);
        assert_eq!(found.render_nodes.len(), 1);
        assert!(!found.render_nodes[0].present);
        assert_eq!(found.connectors.len(), 2);
        assert_eq!(found.connectors[0].modes, ["1920x1080", "1280x720"]);
        assert!(found.connectors[1].modes.is_empty());
        assert_eq!(found.input_nodes.len(), 1);
        assert!(found.warnings.is_empty());
    }

    #[test]
    fn missing_sysfs_is_diagnostic_not_fatal() {
        let fixture = tempfile::tempdir().unwrap();
        let found = Inventory::scan(&fixture.path().join("sys"), &fixture.path().join("dev"));
        assert!(found.cards.is_empty());
        assert_eq!(found.warnings.len(), 2);
    }
}
