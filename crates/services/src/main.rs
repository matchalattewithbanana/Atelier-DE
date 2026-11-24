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
        [arg] if arg == "--help" || arg == "-h" => println!("atelier-services power | media\natelier-services media-action PLAYER play|pause|play-pause|next|previous\nVersioned snapshots and explicit media controls; media requires playerctl."),
        _ => bail!("usage: atelier-services power | media | media-action PLAYER ACTION"),
    }
    Ok(())
}
