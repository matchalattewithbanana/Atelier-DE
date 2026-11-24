//! Entry point. The nested backend is intentionally opt-in until TTY support lands.

mod nested;
mod state;
mod layout;
mod hardware;

use anyhow::{bail, Context, Result};
use atelier_config::{config_path, Config};

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let mut args = std::env::args().skip(1);
    let mut socket = None;
    let mut nested = false;
    let mut probe = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--nested" => nested = true,
            "--probe-hardware" => probe = true,
            "--socket" => socket = Some(args.next().context("--socket requires a name")?),
            "--help" | "-h" => {
                println!("atelier-compositor --nested [--socket NAME]\natelier-compositor --probe-hardware\nDevelopment Wayland compositor (requires an existing Wayland or X11 session).");
                return Ok(());
            }
            other => bail!("unknown argument {other}"),
        }
    }
    if probe {
        if nested || socket.is_some() { bail!("--probe-hardware cannot be combined with session arguments"); }
        hardware::report();
        return Ok(());
    }
    if !nested { bail!("only the --nested backend is implemented in this milestone"); }
    let socket = socket.unwrap_or_else(|| format!("wayland-atelier-{}", std::process::id()));
    if socket.is_empty() || socket.len() > 64 || !socket.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        bail!("invalid Wayland socket name");
    }
    let config = Config::load(&config_path()?)?;
    nested::run(&socket, config.theme.background(), config.layout)
}
