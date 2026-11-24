//! Entry point. The nested backend is intentionally opt-in until TTY support lands.

mod nested;
mod state;
mod layout;
mod hardware;
#[cfg(feature = "native-seat")]
mod seat;
#[cfg(feature = "native-gbm")]
mod gbm_check;

use anyhow::{bail, Context, Result};
use atelier_config::{config_path, Config};

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let mut args = std::env::args().skip(1);
    let mut socket = None;
    let mut nested = false;
    let mut probe = false;
    let mut check_seat = false;
    let mut check_gbm = false;
    let mut check_render = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--nested" => nested = true,
            "--probe-hardware" => probe = true,
            "--check-seat" => check_seat = true,
            "--check-gbm" => check_gbm = true,
            "--check-render" => check_render = true,
            "--socket" => socket = Some(args.next().context("--socket requires a name")?),
            "--help" | "-h" => {
                println!("atelier-compositor --nested [--socket NAME]\natelier-compositor --probe-hardware\natelier-compositor --check-seat (requires native-seat feature and local text login)\natelier-compositor --check-gbm (requires native-gbm feature and local text login)\natelier-compositor --check-render (requires native-gbm feature and local text login)\nDevelopment Wayland compositor (requires an existing Wayland or X11 session).");
                return Ok(());
            }
            other => bail!("unknown argument {other}"),
        }
    }
    if check_render {
        if nested || probe || check_seat || check_gbm || socket.is_some() { bail!("--check-render cannot be combined with other modes"); }
        #[cfg(feature = "native-gbm")]
        return seat::check(true, true);
        #[cfg(not(feature = "native-gbm"))]
        bail!("rebuild with --features native-gbm to check GPU rendering");
    }
    if check_gbm {
        if nested || probe || check_seat || socket.is_some() { bail!("--check-gbm cannot be combined with other modes"); }
        #[cfg(feature = "native-gbm")]
        return seat::check(true, false);
        #[cfg(not(feature = "native-gbm"))]
        bail!("rebuild with --features native-gbm to check GPU allocation");
    }
    if check_seat {
        if nested || probe || socket.is_some() { bail!("--check-seat cannot be combined with other modes"); }
        #[cfg(feature = "native-seat")]
        return seat::check(false, false);
        #[cfg(not(feature = "native-seat"))]
        bail!("rebuild with --features native-seat to check a seat");
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
