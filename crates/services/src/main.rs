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
        [arg] if arg == "--help" || arg == "-h" => println!("atelier-services power | media | audio\natelier-services media-action PLAYER play|pause|play-pause|next|previous\natelier-services audio-action output|input DEVICE volume PERCENT|mute on|off|default\nVersioned snapshots and explicit controls; audio requires pactl."),
        _ => bail!("usage: atelier-services --help"),
    }
    Ok(())
}
