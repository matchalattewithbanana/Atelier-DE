use anyhow::{bail, Context, Result};
use atelier_services::system_power_snapshot;
use std::env;

fn main() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    match args.as_slice() {
        [arg] if arg == "power" => {
            let snapshot = system_power_snapshot().context("read Linux power supply inventory")?;
            println!("{}", serde_json::to_string(&snapshot)?);
        }
        [arg] if arg == "media" => println!("{}", serde_json::to_string(&atelier_services::media::media_snapshot()?)?),
        [mode, player, action] if mode == "media-action" => {
            atelier_services::media::media_action(player, atelier_services::media::MediaAction::parse(action)?)?;
        }
        [arg] if arg == "audio" => println!("{}", serde_json::to_string(&atelier_services::audio::audio_snapshot()?)?),
        [mode, direction, device, action, values @ ..] if mode == "audio-action" => {
            use atelier_services::audio::{audio_action, AudioAction, Direction};
            let action = match (action.as_str(), values) {
                ("volume", [value]) => AudioAction::Volume(value.parse::<u8>().context("expected volume 0..100")?),
                ("mute", [value]) if value == "on" || value == "off" => AudioAction::Mute(value == "on"),
                ("default", []) => AudioAction::Default,
                _ => bail!("expected volume PERCENT, mute on|off, or default"),
            };
            audio_action(Direction::parse(direction)?, device, action)?;
        }
        [arg] if arg == "network" => println!("{}", serde_json::to_string(&atelier_services::network::network_snapshot()?)?),
        [arg] if arg == "wifi" => println!("{}", serde_json::to_string(&atelier_services::network::wifi_snapshot(false)?)?),
        [arg, flag] if arg == "wifi" && flag == "--rescan" => println!("{}", serde_json::to_string(&atelier_services::network::wifi_snapshot(true)?)?),
        [mode, action, value] if mode == "network-action" => {
            use atelier_services::network::{network_action, NetworkAction};
            let action = match action.as_str() {
                "wifi" if value == "on" || value == "off" => NetworkAction::Wifi(value == "on"),
                "activate" => NetworkAction::Activate(value.clone()),
                "deactivate" => NetworkAction::Deactivate(value.clone()),
                _ => bail!("expected wifi on|off, activate UUID, or deactivate UUID"),
            };
            network_action(action)?;
        }
        [arg] if arg == "bluetooth" => println!("{}", serde_json::to_string(&atelier_services::bluetooth::bluetooth_snapshot()?)?),
        [mode, action, value] if mode == "bluetooth-action" => {
            use atelier_services::bluetooth::{bluetooth_action, BluetoothAction};
            let action = match action.as_str() {
                "power" if value == "on" || value == "off" => BluetoothAction::Power(value == "on"),
                "connect" => BluetoothAction::Connect(value.clone()),
                "disconnect" => BluetoothAction::Disconnect(value.clone()),
                _ => bail!("expected power on|off, connect ADDRESS, or disconnect ADDRESS"),
            };
            bluetooth_action(action)?;
        }
        [arg] if arg == "--help" || arg == "-h" => println!("atelier-services power | media | audio | network | wifi [--rescan] | bluetooth\natelier-services media-action PLAYER play|pause|play-pause|next|previous\natelier-services audio-action output|input DEVICE volume PERCENT|mute on|off|default\natelier-services network-action wifi on|off | activate UUID | deactivate UUID\natelier-services bluetooth-action power on|off | connect ADDRESS | disconnect ADDRESS\nVersioned snapshots and explicit controls; optional providers require their respective tools."),
        _ => bail!("usage: atelier-services --help"),
    }
    Ok(())
}
