//! User-session supervisor; owns only the child processes that it starts.

use anyhow::{bail, Context, Result};
mod profiles;
use atelier_config::{component, config_path, Config};
use std::{env, fs, os::unix::{fs::FileTypeExt, net::UnixStream}, path::PathBuf, process::{Child, Command}, sync::{atomic::{AtomicBool, Ordering}, Arc}, thread, time::{Duration, Instant}};

fn main() -> Result<()> {
    let mut args = env::args().skip(1).peekable();
    if args.peek().is_some_and(|arg| arg == "--help" || arg == "-h") {
        println!("atelier-session (--nested | --native) [--no-shell | --shell-command PROGRAM [--shell-arg ARG]... | --profile NAME]\natelier-session --list-profiles\nStarts an Atelier session with the default shell, no shell, or an explicitly selected external shell. Native mode is experimental and requires a spare local TTY.");
        return Ok(());
    }
    if args.peek().is_some_and(|arg| arg == "--list-profiles") {
        args.next();
        if args.next().is_some() { bail!("--list-profiles must be used alone"); }
        let profiles = profiles::load()?;
        if profiles.is_empty() { println!("No saved shell profiles."); }
        for (name, shell) in profiles { println!("{name}: {}", shell.program); }
        return Ok(());
    }
    let options = Options::parse(args)?;
    let config = Config::load(&config_path()?)?;
    let explicit = match options.profile.as_deref() {
        Some(name) => Some(profiles::selected(name)?),
        None => options.shell,
    };
    let shell = explicit.or_else(|| {
        if !options.no_shell && config.start_shell { Some(Shell { program: "atelier-shell".into(), args: Vec::new() }) } else { None }
    });
    let running = Arc::new(AtomicBool::new(true));
    let signal_flag = Arc::clone(&running);
    ctrlc::set_handler(move || signal_flag.store(false, Ordering::SeqCst))?;
    let runtime = PathBuf::from(env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is required")?);
    if !runtime.is_absolute() || !runtime.is_dir() { bail!("XDG_RUNTIME_DIR must be an existing absolute directory"); }
    let socket = format!("wayland-atelier-{}", std::process::id());
    let path = runtime.join(&socket);
    if fs::symlink_metadata(&path).is_ok() { bail!("session socket already exists: {}", path.display()); }

    let cursor=if options.backend=="--native"{
        atelier_config::appearance_path().and_then(|path|atelier_config::Appearance::load(&path))
            .map(|appearance|appearance.cursor).unwrap_or_else(|error|{eprintln!("Cursor preferences unavailable: {error:#}");Default::default()})
    }else{atelier_config::CursorPreferences::default()};
    let mut children = Children::default();
    let mut compositor=component("atelier-compositor");
    compositor.args([options.backend,"--socket",&socket]);
    cursor.apply_to(&mut compositor);
    children.compositor=Some(compositor.spawn().context("start atelier-compositor (build the workspace first)")?);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if !running.load(Ordering::SeqCst) { return Ok(()); }
        if let Some(status) = children.compositor.as_mut().unwrap().try_wait()? { bail!("compositor exited before readiness: {status}"); }
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_socket()) && UnixStream::connect(&path).is_ok() { break; }
        if Instant::now() >= deadline { bail!("compositor did not create a listening Wayland socket within 10 seconds"); }
        thread::sleep(Duration::from_millis(50));
    }

    println!("Atelier session ready; WAYLAND_DISPLAY={socket}");
    if let Some(profile) = &shell {
        let mut command=wayland_child(profile,&socket);cursor.apply_to(&mut command);
        children.shell=Some(command.spawn().context("start selected shell")?);
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
                    let mut command=wayland_child(shell.as_ref().context("missing shell profile")?,&socket);cursor.apply_to(&mut command);
                    children.shell=Some(command.spawn().context("restart selected shell")?);
                } else {
                    eprintln!("Shell restart limit reached; compositor remains active");
                }
            }
        }
        thread::sleep(Duration::from_millis(150));
    }
    Ok(())
}

fn wayland_child(profile: &Shell, socket: &str) -> Command {
    let mut command = component(&profile.program);
    command.args(&profile.args);
    command.env("WAYLAND_DISPLAY", socket).env("GDK_BACKEND", "wayland")
        .env("XDG_SESSION_TYPE", "wayland").env("XDG_CURRENT_DESKTOP", "Atelier")
        .env_remove("WAYLAND_SOCKET").env_remove("DISPLAY")
        .env_remove("MANGO_INSTANCE_SIGNATURE").env_remove("HYPRLAND_INSTANCE_SIGNATURE")
        .env_remove("NIRI_SOCKET").env_remove("ATELIER_COMPOSITOR")
        .env("XDG_SESSION_DESKTOP", "Atelier");
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

#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Shell { program: String, #[serde(default)] args: Vec<String> }

#[derive(Debug)]
struct Options { backend: &'static str, no_shell: bool, shell: Option<Shell>, profile: Option<String> }

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut backend = None;
        let mut no_shell = false;
        let mut shell = None;
        let mut profile = None;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--nested" | "--native" => {
                    if backend.is_some() { bail!("select exactly one backend"); }
                    backend = Some(if arg == "--nested" { "--nested" } else { "--native" });
                }
                "--no-shell" => {
                    if no_shell || shell.is_some() || profile.is_some() { bail!("select exactly one shell profile"); }
                    no_shell = true;
                }
                "--shell-command" => {
                    if no_shell || shell.is_some() || profile.is_some() { bail!("select exactly one shell profile"); }
                    let program = args.next().context("--shell-command requires an executable")?;
                    if program.trim().is_empty() || program.starts_with('-') { bail!("invalid shell executable"); }
                    shell = Some(Shell { program, args: Vec::new() });
                }
                "--profile" => {
                    if no_shell || shell.is_some() || profile.is_some() { bail!("select exactly one shell profile"); }
                    profile = Some(args.next().context("--profile requires a name")?);
                }
                "--shell-arg" => {
                    let profile = shell.as_mut().context("--shell-arg requires a preceding --shell-command")?;
                    profile.args.push(args.next().context("--shell-arg requires a value")?);
                }
                other => bail!("unknown argument {other}"),
            }
        }
        Ok(Self { backend: backend.context("select --nested or --native")?, no_shell, shell, profile })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<Options> { Options::parse(args.iter().map(|s| s.to_string())) }

    #[test]
    fn profiles_are_explicit_and_arguments_are_literal() {
        let opts = parse(&["--nested", "--shell-command", "quickshell", "--shell-arg", "-c", "--shell-arg", "my profile"]).unwrap();
        assert_eq!(opts.backend, "--nested");
        assert_eq!(opts.shell.unwrap(), Shell { program: "quickshell".into(), args: vec!["-c".into(), "my profile".into()] });
        assert!(parse(&["--native", "--no-shell"]).unwrap().no_shell);
        assert_eq!(parse(&["--nested", "--profile", "classic"]).unwrap().profile.as_deref(), Some("classic"));
    }

    #[test]
    fn rejects_conflicting_or_incomplete_options() {
        for args in [vec!["--nested", "--native"], vec!["--nested", "--no-shell", "--shell-command", "foo"], vec!["--nested", "--shell-arg", "foo"], vec!["--nested", "--shell-command"], vec!["--nested", "--profile", "classic", "--no-shell"], vec!["--nested", "--profile", "classic", "--shell-arg", "bad"], vec![]] {
            assert!(parse(&args).is_err(), "accepted {args:?}");
        }
    }
}
