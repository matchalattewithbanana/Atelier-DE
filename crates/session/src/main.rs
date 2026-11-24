//! User-session supervisor; owns only the child processes that it starts.

use anyhow::{bail, Context, Result};
use atelier_config::{component, config_path, Config};
use std::{env, fs, os::unix::{fs::FileTypeExt, net::UnixStream}, path::PathBuf, process::{Child, Command}, sync::{atomic::{AtomicBool, Ordering}, Arc}, thread, time::{Duration, Instant}};

fn main() -> Result<()> {
    let mut start_shell = true;
    let mut nested = false;
    for arg in env::args().skip(1) {
        match arg.as_str() {
            "--nested" => nested = true,
            "--no-shell" => start_shell = false,
            "--help" | "-h" => {
                println!("atelier-session --nested [--no-shell]\nStarts and supervises the native Atelier development session.");
                return Ok(());
            }
            other => bail!("unknown argument {other}"),
        }
    }
    if !nested { bail!("native DRM session is not implemented yet; use --nested"); }
    let config = Config::load(&config_path()?)?;
    let running = Arc::new(AtomicBool::new(true));
    let signal_flag = Arc::clone(&running);
    ctrlc::set_handler(move || signal_flag.store(false, Ordering::SeqCst))?;
    let runtime = PathBuf::from(env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is required")?);
    if !runtime.is_absolute() || !runtime.is_dir() { bail!("XDG_RUNTIME_DIR must be an existing absolute directory"); }
    let socket = format!("wayland-atelier-{}", std::process::id());
    let path = runtime.join(&socket);
    if fs::symlink_metadata(&path).is_ok() { bail!("session socket already exists: {}", path.display()); }

    let mut children = Children::default();
    children.compositor = Some(component("atelier-compositor").args(["--nested", "--socket", &socket]).spawn().context("start atelier-compositor (build the workspace first)")?);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if !running.load(Ordering::SeqCst) { return Ok(()); }
        if let Some(status) = children.compositor.as_mut().unwrap().try_wait()? { bail!("compositor exited before readiness: {status}"); }
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_socket()) && UnixStream::connect(&path).is_ok() { break; }
        if Instant::now() >= deadline { bail!("compositor did not create a listening Wayland socket within 10 seconds"); }
        thread::sleep(Duration::from_millis(50));
    }

    println!("Atelier session ready; WAYLAND_DISPLAY={socket}");
    if start_shell && config.start_shell {
        children.shell = Some(wayland_child("atelier-shell", &socket).spawn().context("start atelier-shell")?);
    }
    let mut restart_times = Vec::new();
    while running.load(Ordering::SeqCst) {
        if let Some(status) = children.compositor.as_mut().unwrap().try_wait()? {
            if !status.success() { bail!("compositor exited: {status}"); }
            return Ok(());
        }
        if let Some(status) = children.shell.as_mut().map(Child::try_wait).transpose()?.flatten() {
            children.shell = None;
            if !status.success() {
                restart_times.retain(|time: &Instant| time.elapsed() < Duration::from_secs(60));
                if restart_times.len() < 3 {
                    restart_times.push(Instant::now());
                    eprintln!("Atelier shell exited ({status}); restarting ({}/3)", restart_times.len());
                    children.shell = Some(wayland_child("atelier-shell", &socket).spawn().context("restart atelier-shell")?);
                } else {
                    eprintln!("Shell restart limit reached; compositor remains active");
                }
            }
        }
        thread::sleep(Duration::from_millis(150));
    }
    Ok(())
}

fn wayland_child(name: &str, socket: &str) -> Command {
    let mut command = component(name);
    command.env("WAYLAND_DISPLAY", socket).env("GDK_BACKEND", "wayland")
        .env("XDG_SESSION_TYPE", "wayland").env("XDG_CURRENT_DESKTOP", "Atelier")
        .env_remove("WAYLAND_SOCKET").env_remove("DISPLAY");
    command
}

#[derive(Default)]
struct Children { compositor: Option<Child>, shell: Option<Child> }
impl Drop for Children {
    fn drop(&mut self) {
        for child in [&mut self.shell, &mut self.compositor].into_iter().flatten() {
            if child.try_wait().ok().flatten().is_none() {
                let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(child.id() as i32), nix::sys::signal::Signal::SIGTERM);
                let deadline = Instant::now() + Duration::from_secs(2);
                while Instant::now() < deadline && child.try_wait().ok().flatten().is_none() {
                    thread::sleep(Duration::from_millis(50));
                }
                if child.try_wait().ok().flatten().is_none() { let _ = child.kill(); }
            }
            let _ = child.wait();
        }
    }
}
