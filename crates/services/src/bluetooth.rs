//! Optional BlueZ provider. Only already-paired devices are managed.
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::process::Command;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct BluetoothSnapshot {
    pub schema: u32,
    pub controllers: Vec<Controller>,
    pub powered: Option<bool>,
    pub paired_devices: Vec<Device>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Controller { pub address: String, pub name: String }

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Device { pub address: String, pub name: String, pub connected: Option<bool> }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BluetoothAction { Power(bool), Connect(String), Disconnect(String) }

fn run(args: &[&str]) -> Result<String> {
    let output = Command::new("timeout").args(["--kill-after=1s", "12s", "bluetoothctl", "--timeout", "10"])
        .args(args).env("LC_ALL", "C").env("TERM", "dumb").output()
        .context("run bluetoothctl through coreutils timeout (both required for Bluetooth support)")?;
    let stdout = String::from_utf8(output.stdout).context("invalid Bluetooth provider output")?;
    if !output.status.success() || stdout.lines().any(|line| {
        let line = line.trim();
        line.starts_with("Failed to ") || line.starts_with("No default controller") || line.starts_with("Invalid command")
    }) {
        bail!("BlueZ command failed ({}): {}", output.status, String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(stdout)
}

fn address_valid(address: &str) -> bool {
    address.len() == 17 && address.bytes().enumerate().all(|(i, byte)| {
        if i % 3 == 2 { byte == b':' } else { byte.is_ascii_hexdigit() }
    })
}

fn entries(text: &str, prefix: &str) -> Result<Vec<(String, String)>> {
    let mut result = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let rest = line.strip_prefix(prefix).context("unexpected BlueZ list output")?;
        let (address, name) = rest.split_once(' ').unwrap_or((rest, ""));
        if !address_valid(address) { bail!("invalid Bluetooth address"); }
        let address = address.to_ascii_uppercase();
        if result.iter().any(|(existing, _)| existing == &address) { bail!("duplicate Bluetooth address"); }
        result.push((address, name.to_owned()));
    }
    Ok(result)
}

fn flag(text: &str, key: &str) -> Option<bool> {
    text.lines().find_map(|line| {
        match line.trim().strip_prefix(key)?.trim() {
            "yes" => Some(true), "no" => Some(false), _ => None,
        }
    })
}

fn controllers(call: &mut impl FnMut(&[&str]) -> Result<String>) -> Result<Vec<Controller>> {
    Ok(entries(&call(&["list"])?, "Controller ")?.into_iter().map(|(address, name)| Controller { address, name }).collect())
}

fn snapshot_with(mut call: impl FnMut(&[&str]) -> Result<String>) -> Result<BluetoothSnapshot> {
    let controllers = controllers(&mut call)?;
    // Multiple adapters require adapter-scoped D-Bus enumeration. Do not infer
    // which adapter bluetoothctl chose for separate process invocations.
    if controllers.len() != 1 {
        return Ok(BluetoothSnapshot { schema: 1, controllers, powered: None, paired_devices: Vec::new() });
    }
    let powered = flag(&call(&["show"])?, "Powered:");
    let paired = entries(&call(&["devices", "Paired"])?, "Device ")?;
    let mut paired_devices = Vec::new();
    for (address, name) in paired {
        let connected = call(&["info", &address]).ok().and_then(|info| flag(&info, "Connected:"));
        paired_devices.push(Device { address, name, connected });
    }
    Ok(BluetoothSnapshot { schema: 1, controllers, powered, paired_devices })
}
pub fn bluetooth_snapshot() -> Result<BluetoothSnapshot> { snapshot_with(run) }

fn action_with(action: BluetoothAction, mut call: impl FnMut(&[&str]) -> Result<String>) -> Result<()> {
    if controllers(&mut call)?.len() != 1 { bail!("Bluetooth controls require exactly one controller"); }
    match action {
        BluetoothAction::Power(enabled) => {
            call(&["power", if enabled { "on" } else { "off" }])?;
            if flag(&call(&["show"])?, "Powered:") != Some(enabled) { bail!("requested Bluetooth power state not confirmed"); }
        }
        action @ (BluetoothAction::Connect(_) | BluetoothAction::Disconnect(_)) => {
            let (address, connect) = match action {
                BluetoothAction::Connect(address) => (address, true),
                BluetoothAction::Disconnect(address) => (address, false),
                _ => unreachable!(),
            };
            if !address_valid(&address) { bail!("invalid Bluetooth address"); }
            let address = address.to_ascii_uppercase();
            if !entries(&call(&["devices", "Paired"])?, "Device ")?.iter().any(|(id, _)| id == &address) {
                bail!("selected device is not paired");
            }
            call(&[if connect { "connect" } else { "disconnect" }, &address])?;
            if flag(&call(&["info", &address])?, "Connected:") != Some(connect) {
                bail!("requested Bluetooth connection state not confirmed");
            }
        }
    }
    Ok(())
}
pub fn bluetooth_action(action: BluetoothAction) -> Result<()> { action_with(action, run) }

#[cfg(test)]
mod tests {
    use super::*;
    const ADAPTER: &str = "Controller 00:11:22:33:44:55 Laptop [default]\n";
    const DEVICE: &str = "Device AA:BB:CC:DD:EE:FF My headphones\n";
    #[test]
    fn inventory_keeps_unknown_state_distinct() {
        let snapshot = snapshot_with(|args| Ok(match args {
            ["list"] => ADAPTER, ["show"] => "\tPowered: yes\n",
            ["devices", "Paired"] => DEVICE, _ => "\tConnected: no\n",
        }.to_owned())).unwrap();
        assert_eq!(snapshot.powered, Some(true));
        assert_eq!(snapshot.paired_devices[0].name, "My headphones");
        assert_eq!(snapshot.paired_devices[0].connected, Some(false));
        assert!(entries("unexpected output", "Device ").is_err());
    }
    #[test]
    fn no_adapter_and_multiple_adapters_do_not_guess() {
        let empty = snapshot_with(|_| Ok(String::new())).unwrap();
        assert!(empty.controllers.is_empty());
        assert_eq!(empty.powered, None);
        let multiple = format!("{ADAPTER}Controller 11:22:33:44:55:66 Other\n");
        assert!(action_with(BluetoothAction::Power(true), |_| Ok(multiple.clone())).is_err());
    }
    #[test]
    fn controls_require_paired_addresses_and_confirm_state() {
        let mut calls = Vec::new();
        action_with(BluetoothAction::Connect("aa:bb:cc:dd:ee:ff".into()), |args| {
            calls.push(args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            Ok(match args { ["list"] => ADAPTER, ["devices", "Paired"] => DEVICE, ["info", _] => "Connected: yes", _ => "" }.to_owned())
        }).unwrap();
        assert_eq!(calls[2], vec!["connect", "AA:BB:CC:DD:EE:FF"]);
        assert!(action_with(BluetoothAction::Disconnect("AA:BB:CC:DD:EE:FF".into()), |args| {
            Ok(if args == ["list"] { ADAPTER } else { "" }.into())
        }).is_err());
        assert!(action_with(BluetoothAction::Power(true), |args| Ok(if args == ["list"] { ADAPTER } else { "Powered: no" }.into())).is_err());
    }
}
