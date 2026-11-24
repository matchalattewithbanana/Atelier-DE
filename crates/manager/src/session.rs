//! Verified launch of an experimental nested session from the selected deployment.
use anyhow::{bail, Context, Result};
use std::{env, path::{Path, PathBuf}, process::Command};
use crate::selection;

fn user_only() -> Result<()> {
    if nix::unistd::Uid::effective().is_root() { bail!("run as your desktop user, not root"); }
    Ok(())
}

fn host_available(wayland: Option<&str>, x11: Option<&str>) -> bool {
    [wayland, x11].into_iter().flatten().any(|value| !value.trim().is_empty())
}

fn host_check() -> Result<()> {
    if !host_available(env::var("WAYLAND_DISPLAY").ok().as_deref(), env::var("DISPLAY").ok().as_deref()) {
        bail!("nested mode needs an existing graphical Wayland or X11 host session");
    }
    let runtime = PathBuf::from(env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is required")?);
    if !runtime.is_absolute() || !runtime.is_dir() { bail!("XDG_RUNTIME_DIR must be an existing absolute directory"); }
    Ok(())
}

fn command(bin: &Path, no_shell: bool) -> Command {
    let mut cmd = Command::new(bin.join("atelier-session"));
    cmd.arg("--nested");
    if no_shell { cmd.arg("--no-shell"); }
    cmd
}

pub fn check() -> Result<()> {
    user_only()?;
    let bin = selection::verified_session_bin(false)?;
    println!("Verified selected session bundle: {}", bin.display());
    host_check()?;
    println!("Host display variables and runtime directory are present.");
    println!("This is a static readiness check, not a GPU, display connection or login readiness test.");
    Ok(())
}

pub fn launch(no_shell: bool) -> Result<()> {
    user_only()?;
    host_check()?;
    let leased = selection::leased_session(no_shell)?;
    println!("Starting experimental nested Atelier from {}", leased.path().display());
    println!("The existing graphical session remains the host; close the nested window to exit.");
    // Replace this CLI process so the existing supervisor owns signals and children.
    let cmd = command(leased.path(), no_shell);
    leased.exec(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_a_host_display_hint() {
        assert!(!host_available(None, Some(" ")));
        assert!(host_available(Some("wayland-0"), None));
        assert!(host_available(None, Some(":0")));
    }
    #[test]
    fn launches_exact_selected_executable_with_nested_mode_only() {
        let cmd = command(Path::new("/trusted/bin"), true);
        assert_eq!(cmd.get_program(), "/trusted/bin/atelier-session");
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), vec!["--nested", "--no-shell"]);
        assert_eq!(command(Path::new("/trusted/bin"), false).get_args().collect::<Vec<_>>(), vec!["--nested"]);
    }
}
