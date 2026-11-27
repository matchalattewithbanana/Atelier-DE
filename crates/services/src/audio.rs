//! Optional audio provider for the PulseAudio protocol (including PipeWire's pulse server).
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::process::Command;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct AudioSnapshot {
    pub schema: u32,
    pub outputs: Vec<Device>,
    pub inputs: Vec<Device>,
    pub default_output: Option<String>,
    pub default_input: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub description: String,
    pub muted: Option<bool>,
    pub channels: Vec<Channel>,
    pub monitor: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Channel { pub name: String, pub percent: Option<u32> }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction { Output, Input }
impl Direction {
    pub fn parse(value: &str) -> Result<Self> {
        match value { "output" => Ok(Self::Output), "input" => Ok(Self::Input), _ => bail!("expected output or input") }
    }
    fn target(self) -> &'static str { match self { Self::Output => "sink", Self::Input => "source" } }
    fn collection(self) -> &'static str { match self { Self::Output => "sinks", Self::Input => "sources" } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioAction { Volume(u8), Mute(bool), Default }

fn run(args: &[&str]) -> Result<String> {
    let output = Command::new("timeout").args(["--kill-after=1s", "3s", "pactl"]).args(args).env("LC_ALL", "C").output()
        .context("run pactl through coreutils timeout (both required for audio support)")?;
    if !output.status.success() {
        bail!("audio provider failed ({}): {}", output.status, String::from_utf8_lossy(&output.stderr).trim());
    }
    String::from_utf8(output.stdout).context("invalid pactl output")
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 255 && !id.starts_with('-') && !id.bytes().all(|b| b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b':'))
}

fn parse_devices(text: &str, direction: Direction) -> Result<Vec<Device>> {
    let data: Vec<Value> = serde_json::from_str(text).context("invalid audio device list")?;
    let mut devices = Vec::new();
    for item in data {
        let id = item.get("name").and_then(Value::as_str).filter(|id| valid_id(id)).context("invalid audio device identifier")?;
        let mut channels = Vec::new();
        if let Some(volume) = item.get("volume").and_then(Value::as_object) {
            for (name, value) in volume {
                let percent = value.get("value_percent").and_then(Value::as_str)
                    .and_then(|s| s.trim().strip_suffix('%')).and_then(|s| s.trim().parse::<u32>().ok());
                channels.push(Channel { name: name.clone(), percent });
            }
        }
        devices.push(Device {
            id: id.to_owned(),
            description: item.get("description").and_then(Value::as_str).unwrap_or(id).to_owned(),
            muted: item.get("mute").and_then(Value::as_bool),
            channels,
            monitor: direction == Direction::Input && id.ends_with(".monitor"),
        });
    }
    devices.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(devices)
}

fn list(direction: Direction, call: &mut impl FnMut(&[&str]) -> Result<String>) -> Result<Vec<Device>> {
    parse_devices(&call(&["--format=json", "list", direction.collection()])?, direction)
}

fn default_device(direction: Direction, devices: &[Device], call: &mut impl FnMut(&[&str]) -> Result<String>) -> Option<String> {
    let command = format!("get-default-{}", direction.target());
    let name = call(&[&command]).ok()?;
    let name = name.trim();
    devices.iter().find(|device| device.id == name).map(|device| device.id.clone())
}

fn snapshot_with(mut call: impl FnMut(&[&str]) -> Result<String>) -> Result<AudioSnapshot> {
    let outputs = list(Direction::Output, &mut call)?;
    let inputs = list(Direction::Input, &mut call)?;
    let default_output = default_device(Direction::Output, &outputs, &mut call);
    let default_input = default_device(Direction::Input, &inputs, &mut call);
    Ok(AudioSnapshot { schema: 1, outputs, inputs, default_output, default_input })
}
pub fn audio_snapshot() -> Result<AudioSnapshot> { snapshot_with(run) }

fn action_with(direction: Direction, id: &str, action: AudioAction, mut call: impl FnMut(&[&str]) -> Result<String>) -> Result<()> {
    checked_action(direction,id,action,false,&mut call)
}
fn checked_action(direction:Direction,id:&str,action:AudioAction,require_default:bool,call:&mut impl FnMut(&[&str])->Result<String>)->Result<()> {
    if !valid_id(id) { bail!("invalid audio device identifier"); }
    if matches!(action, AudioAction::Volume(value) if value > 100) { bail!("volume must be between 0 and 100 percent"); }
    let devices=list(direction,call)?;
    if !devices.iter().any(|device|device.id==id){bail!("selected audio device is unavailable");}
    if require_default&&default_device(direction,&devices,call).as_deref()!=Some(id){bail!("default audio device changed; review the new device");}
    let target = direction.target();
    match action {
        AudioAction::Volume(value) => call(&[&format!("set-{target}-volume"), id, &format!("{value}%")])?,
        AudioAction::Mute(value) => call(&[&format!("set-{target}-mute"), id, if value { "1" } else { "0" }])?,
        AudioAction::Default => call(&[&format!("set-default-{target}"), id])?,
    };
    Ok(())
}
pub fn audio_action(direction: Direction, id: &str, action: AudioAction) -> Result<()> { action_with(direction, id, action, run) }
/// Recheck the reviewed default before a volume or mute write. The named
/// target is still explicit; pactl does not offer an atomic compare-and-set.
pub fn audio_default_action(direction:Direction,id:&str,action:AudioAction)->Result<()>{
    if action==AudioAction::Default{bail!("expected a volume or mute action");}
    checked_action(direction,id,action,true,&mut run)
}

#[cfg(test)]
mod tests {
    use super::*;
    const SINKS: &str = r#"[{"name":"alsa_output.test","description":"Speakers","mute":false,"volume":{"front-left":{"value_percent":" 72%"},"front-right":{"value_percent":"125%"}}}]"#;
    #[test]
    fn parses_channel_volumes_and_matches_default_by_name() {
        let result = snapshot_with(|args| Ok(match args {
            ["--format=json", "list", "sinks"] => SINKS,
            ["--format=json", "list", "sources"] => r#"[{"name":"alsa_output.test.monitor","mute":true}]"#,
            ["get-default-sink"] => "alsa_output.test\n",
            _ => "missing",
        }.to_owned())).unwrap();
        assert_eq!(result.default_output.as_deref(), Some("alsa_output.test"));
        assert_eq!(result.default_input, None);
        assert_eq!(result.outputs[0].channels[0].percent, Some(72));
        assert_eq!(result.outputs[0].channels[1].percent, Some(125));
        assert!(result.inputs[0].monitor);
    }
    #[test]
    fn rejects_invalid_inventory_instead_of_reporting_empty() {
        assert!(parse_devices("invalid", Direction::Input).is_err());
        assert!(parse_devices(r#"[{"name":"@DEFAULT_SOURCE@"}]"#, Direction::Input).is_err());
        assert!(parse_devices("[]", Direction::Input).unwrap().is_empty());
    }
    #[test]
    fn reviewed_default_action_refuses_changed_or_unavailable_default(){
        for default in ["other",""]{
            let mut calls=0;
            let result=checked_action(Direction::Output,"alsa_output.test",AudioAction::Volume(45),true,&mut |args|{
                calls+=1;
                match args{["--format=json","list","sinks"]=>Ok(SINKS.into()),["get-default-sink"]=>Ok(default.into()),_=>panic!("must not write")}
            });
            assert!(result.is_err());assert_eq!(calls,2);
        }
        let mut writes=0;
        checked_action(Direction::Output,"alsa_output.test",AudioAction::Mute(true),true,&mut |args|{
            match args{["--format=json","list","sinks"]=>Ok(SINKS.into()),["get-default-sink"]=>Ok("alsa_output.test".into()),["set-sink-mute","alsa_output.test","1"]=>{writes+=1;Ok(String::new())},_=>panic!("unexpected call")}
        }).unwrap();
        assert_eq!(writes,1);
    }
    #[test]
    fn controls_only_discovered_named_devices_and_limits_volume() {
        assert!(action_with(Direction::Output, "alsa_output.test", AudioAction::Volume(101), |_| panic!("must not invoke provider")).is_err());
        assert!(action_with(Direction::Output, "missing", AudioAction::Mute(true), |_| Ok(SINKS.into())).is_err());
        let mut calls = Vec::new();
        action_with(Direction::Output, "alsa_output.test", AudioAction::Volume(45), |args| {
            calls.push(args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            Ok(SINKS.into())
        }).unwrap();
        assert_eq!(calls[1], vec!["set-sink-volume", "alsa_output.test", "45%"]);
    }
}
