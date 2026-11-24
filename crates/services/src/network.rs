//! Optional NetworkManager provider. Never reads passwords or rewrites profiles.
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::process::Command;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct NetworkSnapshot {
    pub schema: u32,
    pub wifi_enabled: Option<bool>,
    pub devices: Vec<Device>,
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub kind: String,
    pub state: String,
    pub connection: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Profile {
    pub uuid: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct AccessPoint {
    pub ssid: Option<String>,
    pub bssid: String,
    pub signal: Option<u8>,
    pub security: Option<String>,
    pub active: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct WifiSnapshot { pub schema: u32, pub access_points: Vec<AccessPoint> }

fn run(args: &[&str]) -> Result<String> {
    let output = Command::new("timeout")
        .args(["--kill-after=1s", "15s", "nmcli", "--wait", "12"])
        .args(args).env("LC_ALL", "C").output()
        .context("run nmcli through coreutils timeout (both required for network support)")?;
    if !output.status.success() {
        bail!("NetworkManager command failed ({}): {}", output.status, String::from_utf8_lossy(&output.stderr).trim());
    }
    String::from_utf8(output.stdout).context("invalid NetworkManager output")
}

/// nmcli terse mode escapes colons and backslashes. Preserve whitespace in names.
fn fields(line: &str, count: usize) -> Result<Vec<String>> {
    let mut fields = vec![String::new()];
    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            if ch != ':' && ch != '\\' { bail!("invalid nmcli escape"); }
            fields.last_mut().unwrap().push(ch);
            escaped = false;
        } else {
            match ch {
                '\\' => escaped = true,
                ':' => fields.push(String::new()),
                _ => fields.last_mut().unwrap().push(ch),
            }
        }
    }
    if escaped || fields.len() != count { bail!("unexpected nmcli field count or escape"); }
    Ok(fields)
}

fn optional(value: String) -> Option<String> {
    if value.is_empty() || value == "--" { None } else { Some(value) }
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36 && value.bytes().enumerate().all(|(i, b)| {
        if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() }
    })
}

fn parse_devices(text: &str) -> Result<Vec<Device>> {
    text.lines().filter(|line| !line.is_empty()).map(|line| {
        let [name, kind, state, connection] = fields(line, 4)?.try_into().map_err(|_| anyhow::anyhow!("invalid device fields"))?;
        if name.is_empty() { bail!("missing device name"); }
        Ok(Device { name, kind, state, connection: optional(connection) })
    }).collect()
}

fn parse_profiles(text: &str) -> Result<Vec<Profile>> {
    text.lines().filter(|line| !line.is_empty()).map(|line| {
        let [uuid, name, kind] = fields(line, 3)?.try_into().map_err(|_| anyhow::anyhow!("invalid profile fields"))?;
        if !valid_uuid(&uuid) { bail!("invalid NetworkManager profile UUID"); }
        Ok(Profile { uuid, name, kind })
    }).collect()
}

fn parse_wifi(text: &str) -> Result<WifiSnapshot> {
    let access_points = text.lines().filter(|line| !line.is_empty()).map(|line| {
        let [active, ssid, bssid, signal, security] = fields(line, 5)?.try_into().map_err(|_| anyhow::anyhow!("invalid access point fields"))?;
        Ok(AccessPoint {
            active: active == "*",
            ssid: optional(ssid), bssid,
            signal: signal.parse::<u8>().ok().filter(|value| *value <= 100),
            security: optional(security),
        })
    }).collect::<Result<Vec<_>>>()?;
    Ok(WifiSnapshot { schema: 1, access_points })
}

fn profiles(call: &mut impl FnMut(&[&str]) -> Result<String>) -> Result<Vec<Profile>> {
    parse_profiles(&call(&["--terse", "--escape", "yes", "--fields", "UUID,NAME,TYPE", "connection", "show"])?)
}

fn snapshot_with(mut call: impl FnMut(&[&str]) -> Result<String>) -> Result<NetworkSnapshot> {
    let devices = parse_devices(&call(&["--terse", "--escape", "yes", "--fields", "DEVICE,TYPE,STATE,CONNECTION", "device", "status"])?)?;
    let profiles = profiles(&mut call)?;
    let radio = call(&["radio", "wifi"])?;
    let wifi_enabled = match radio.trim() { "enabled" => Some(true), "disabled" => Some(false), _ => None };
    Ok(NetworkSnapshot { schema: 1, wifi_enabled, devices, profiles })
}
pub fn network_snapshot() -> Result<NetworkSnapshot> { snapshot_with(run) }

pub fn wifi_snapshot(rescan: bool) -> Result<WifiSnapshot> {
    parse_wifi(&run(&["--terse", "--escape", "yes", "--fields", "IN-USE,SSID,BSSID,SIGNAL,SECURITY",
        "device", "wifi", "list", "--rescan", if rescan { "yes" } else { "no" }])?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkAction { Wifi(bool), Activate(String), Deactivate(String) }

fn action_with(action: NetworkAction, mut call: impl FnMut(&[&str]) -> Result<String>) -> Result<()> {
    match action {
        NetworkAction::Wifi(enabled) => { call(&["radio", "wifi", if enabled { "on" } else { "off" }])?; }
        action @ (NetworkAction::Activate(_) | NetworkAction::Deactivate(_)) => {
            let (command, uuid) = match action {
                NetworkAction::Activate(uuid) => ("up", uuid),
                NetworkAction::Deactivate(uuid) => ("down", uuid),
                _ => unreachable!(),
            };
            if !valid_uuid(&uuid) { bail!("expected a saved connection UUID"); }
            if !profiles(&mut call)?.iter().any(|profile| profile.uuid == uuid) { bail!("selected connection profile is unavailable"); }
            call(&["connection", command, "uuid", &uuid])?;
        }
    }
    Ok(())
}
pub fn network_action(action: NetworkAction) -> Result<()> { action_with(action, run) }

#[cfg(test)]
mod tests {
    use super::*;
    const UUID: &str = "11111111-2222-3333-4444-555555555555";
    #[test]
    fn preserves_escaped_fields_and_hidden_ssids() {
        let devices = parse_devices(r"wlan0:wifi:connected:Home\:Office\\Guest").unwrap();
        assert_eq!(devices[0].connection.as_deref(), Some(r"Home:Office\Guest"));
        let wifi = parse_wifi(r"*::AA\:BB\:CC\:DD\:EE\:FF:86:WPA2").unwrap();
        assert_eq!(wifi.access_points[0].ssid, None);
        assert_eq!(wifi.access_points[0].bssid, "AA:BB:CC:DD:EE:FF");
        assert_eq!(wifi.access_points[0].signal, Some(86));
        assert!(wifi.access_points[0].active);
        assert!(fields(r"foo\q:bar", 2).is_err());
        assert!(parse_devices("wrong:field:count").is_err());
    }
    #[test]
    fn actions_accept_only_saved_uuid_and_literal_arguments() {
        assert!(action_with(NetworkAction::Activate("--help".into()), |_| panic!("invalid UUID must not call provider")).is_err());
        assert!(action_with(NetworkAction::Deactivate(UUID.into()), |_| Ok(String::new())).is_err());
        let mut calls = Vec::new();
        action_with(NetworkAction::Activate(UUID.into()), |args| {
            calls.push(args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            Ok(format!("{UUID}:Saved\\:WiFi:802-11-wireless"))
        }).unwrap();
        assert_eq!(calls[1], vec!["connection", "up", "uuid", UUID]);
    }
    #[test]
    fn distinguishes_provider_failure_from_empty_inventory() {
        assert!(snapshot_with(|_| bail!("offline provider")).is_err());
        let snapshot = snapshot_with(|args| Ok(if args == ["radio", "wifi"] { "disabled" } else { "" }.into())).unwrap();
        assert!(snapshot.devices.is_empty());
        assert!(snapshot.profiles.is_empty());
        assert_eq!(snapshot.wifi_enabled, Some(false));
    }
}
