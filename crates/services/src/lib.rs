//! Compositor-independent desktop service providers.
pub mod media;
use serde::Serialize;
use std::{fs, io, path::Path};

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PowerSnapshot {
    pub schema: u32,
    pub batteries: Vec<Battery>,
    /// None means no readable external power status was available.
    pub external_power: Option<bool>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Battery {
    pub name: String,
    pub percent: Option<u8>,
    pub status: Option<String>,
}

fn value(path: &Path, key: &str) -> Option<String> {
    let value = fs::read_to_string(path.join(key)).ok()?;
    let value = value.trim();
    if value.is_empty() { None } else { Some(value.to_owned()) }
}

pub fn system_power_snapshot() -> io::Result<PowerSnapshot> {
    power_snapshot(Path::new("/sys/class/power_supply"))
}

pub fn power_snapshot(root: &Path) -> io::Result<PowerSnapshot> {
    let mut result = PowerSnapshot { schema: 1, batteries: Vec::new(), external_power: None };
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        match value(&path, "type").as_deref() {
            Some("Battery") => {
                if value(&path, "present").as_deref() == Some("0") { continue; }
                // Peripheral batteries are not laptop power sources.
                if value(&path, "scope").as_deref() == Some("Device") { continue; }
                result.batteries.push(Battery {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    percent: value(&path, "capacity").and_then(|s| s.parse::<u8>().ok()).filter(|p| *p <= 100),
                    status: value(&path, "status"),
                });
            }
            Some("Mains" | "USB" | "USB_C" | "USB_PD" | "USB_PD_DRP" | "Wireless") => {
                if value(&path, "scope").as_deref() == Some("Device") { continue; }
                let online = match value(&path, "online").as_deref() {
                    Some("1") => true,
                    Some("0") => false,
                    _ => continue,
                };
                result.external_power = Some(result.external_power.unwrap_or(false) || online);
            }
            _ => {}
        }
    }
    result.batteries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn supply(root: &Path, name: &str, fields: &[(&str, &str)]) {
        let dir = root.join(name);
        fs::create_dir(&dir).unwrap();
        for (key, value) in fields { fs::write(dir.join(key), value).unwrap(); }
    }
    #[test]
    fn desktop_without_battery_and_unknown_power() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(power_snapshot(dir.path()).unwrap(), PowerSnapshot { schema: 1, batteries: vec![], external_power: None });
    }
    #[test]
    fn separates_system_batteries_and_handles_invalid_fields() {
        let dir = tempfile::tempdir().unwrap();
        supply(dir.path(), "BAT1", &[("type","Battery"), ("capacity","101"), ("status","Unknown")]);
        supply(dir.path(), "BAT0", &[("type","Battery"), ("capacity"," 73\n"), ("status","Charging")]);
        supply(dir.path(), "mouse", &[("type","Battery"), ("scope","Device"), ("capacity","90")]);
        supply(dir.path(), "empty", &[("type","Battery"), ("present","0")]);
        supply(dir.path(), "AC", &[("type","Mains"), ("online","0")]);
        supply(dir.path(), "USB", &[("type","USB_C"), ("online","1")]);
        let result = power_snapshot(dir.path()).unwrap();
        assert_eq!(result.batteries.len(), 2);
        assert_eq!(result.batteries[0].percent, Some(73));
        assert_eq!(result.batteries[1].percent, None);
        assert_eq!(result.external_power, Some(true));
    }
    #[test]
    fn unreadable_inventory_is_not_a_false_empty_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        assert!(power_snapshot(&dir.path().join("missing")).is_err());
    }
}
