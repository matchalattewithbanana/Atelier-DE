//! Entry point. The nested backend is intentionally opt-in until TTY support lands.

mod nested;
mod state;
mod layout;
mod scene;
mod motion;
mod frame_metrics;
mod workspace_control;
#[cfg(any(feature = "native-session", test))]
mod frame_queue;
#[cfg(any(feature = "native-session", test))]
mod client_damage;
#[cfg(feature = "native-session")]
mod native_cursor;
mod input_router;
mod session_lock;
#[cfg(feature = "native-input")]
mod native_input;
mod hardware;
#[cfg(feature = "native-seat")]
mod seat;
#[cfg(feature = "native-gbm")]
mod gbm_check;
#[cfg(feature = "native-kms")]
mod kms_check;

use anyhow::{bail, Context, Result};
use atelier_config::{config_path, Config};

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let mut args = std::env::args().skip(1);
    let mut socket = None;
    let mut nested = false;
    let mut native = false;
    let mut check_input = false;
    let mut probe = false;
    let mut check_seat = false;
    let mut check_gbm = false;
    let mut check_render = false;
    let mut check_kms = false;
    let mut check_atomic = false;
    let mut present_test = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--nested" => nested = true,
            "--native" => native = true,
            "--check-input" => check_input = true,
            "--probe-hardware" => probe = true,
            "--check-seat" => check_seat = true,
            "--check-gbm" => check_gbm = true,
            "--check-render" => check_render = true,
            "--check-kms" => check_kms = true,
            "--check-atomic" => check_atomic = true,
            "--present-test" => present_test = true,
            "--socket" => socket = Some(args.next().context("--socket requires a name")?),
            "--help" | "-h" => {
                println!("atelier-compositor --nested [--socket NAME]\natelier-compositor --native [--socket NAME] (experimental native-session feature)\natelier-compositor --check-input (native-input feature)\natelier-compositor --probe-hardware\natelier-compositor --check-seat (requires native-seat feature and local text login)\natelier-compositor --check-gbm (requires native-gbm feature and local text login)\natelier-compositor --check-render (requires native-gbm feature and local text login)\natelier-compositor --check-kms (requires native-kms feature and local text login)\natelier-compositor --check-atomic (requires native-kms feature and local text login)\natelier-compositor --present-test (EXPERIMENTAL, spare local TTY only)\nDevelopment Wayland compositor (requires an existing Wayland or X11 session).");
                return Ok(());
            }
            other => bail!("unknown argument {other}"),
        }
    }
    if native {
        if nested || probe || check_input || check_seat || check_gbm || check_render || check_kms || check_atomic || present_test { bail!("--native cannot be combined with other modes"); }
        #[cfg(feature = "native-session")]
        {
            let socket = socket.unwrap_or_else(|| format!("wayland-atelier-native-{}", std::process::id()));
            if socket.is_empty() || socket.len() > 64 || !socket.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') { bail!("invalid Wayland socket name"); }
            let config = Config::load(&config_path()?)?;
            return seat::check(seat::Check::Native { socket, config });
        }
        #[cfg(not(feature = "native-session"))]
        bail!("rebuild with --features native-session to run the native compositor");
    }
    if check_input {
        if nested || probe || check_seat || check_gbm || check_render || check_kms || check_atomic || present_test || socket.is_some() { bail!("--check-input cannot be combined with other modes"); }
        #[cfg(feature = "native-input")]
        return seat::check(seat::Check::Input);
        #[cfg(not(feature = "native-input"))]
        bail!("rebuild with --features native-input to check libinput");
    }
    if present_test {
        if nested || probe || check_seat || check_gbm || check_render || check_kms || check_atomic || socket.is_some() { bail!("--present-test cannot be combined with other modes"); }
        #[cfg(feature = "native-kms")]
        return seat::check(seat::Check::Present);
        #[cfg(not(feature = "native-kms"))]
        bail!("rebuild with --features native-kms to test presentation");
    }
    if check_atomic {
        if nested || probe || check_seat || check_gbm || check_render || check_kms || socket.is_some() { bail!("--check-atomic cannot be combined with other modes"); }
        #[cfg(feature = "native-kms")]
        return seat::check(seat::Check::Atomic);
        #[cfg(not(feature = "native-kms"))]
        bail!("rebuild with --features native-kms to validate an atomic route");
    }
    if check_kms {
        if nested || probe || check_seat || check_gbm || check_render || socket.is_some() { bail!("--check-kms cannot be combined with other modes"); }
        #[cfg(feature = "native-kms")]
        return seat::check(seat::Check::Kms);
        #[cfg(not(feature = "native-kms"))]
        bail!("rebuild with --features native-kms to query KMS topology");
    }
    if check_render {
        if nested || probe || check_seat || check_gbm || socket.is_some() { bail!("--check-render cannot be combined with other modes"); }
        #[cfg(feature = "native-gbm")]
        return seat::check(seat::Check::Render);
        #[cfg(not(feature = "native-gbm"))]
        bail!("rebuild with --features native-gbm to check GPU rendering");
    }
    if check_gbm {
        if nested || probe || check_seat || socket.is_some() { bail!("--check-gbm cannot be combined with other modes"); }
        #[cfg(feature = "native-gbm")]
        return seat::check(seat::Check::Gbm);
        #[cfg(not(feature = "native-gbm"))]
        bail!("rebuild with --features native-gbm to check GPU allocation");
    }
    if check_seat {
        if nested || probe || socket.is_some() { bail!("--check-seat cannot be combined with other modes"); }
        #[cfg(feature = "native-seat")]
        return seat::check(seat::Check::Seat);
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
