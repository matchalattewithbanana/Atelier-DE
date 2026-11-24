//! Conservative Arch-family package candidates and local dependency checks.
use crate::{catalog::Plan, Report};
use anyhow::{bail, Context, Result};
use std::{fs, io::{self, IsTerminal, Write}, os::unix::fs::{MetadataExt, PermissionsExt}, path::{Path, PathBuf}};
use serde::Serialize;
use std::{collections::{BTreeMap, BTreeSet}, process::{Command, Output}};

#[derive(Debug, Serialize)]
pub struct Candidate { pub package: String, pub reasons: Vec<String>, pub state: &'static str }
#[derive(Debug, Serialize)]
pub struct PackagePreview {
    pub schema: u32,
    pub backend: Option<&'static str>,
    pub candidates: Vec<Candidate>,
    pub notes: Vec<String>,
    pub apply_supported: bool,
}
fn arch_family(report: &Report) -> bool {
    report.distribution.id.as_deref().is_some_and(|id| matches!(id, "arch" | "cachyos" | "endeavouros"))
        || report.distribution.id_like.iter().any(|id| id == "arch")
}
fn add(map: &mut BTreeMap<String, BTreeSet<String>>, reason: &str, packages: &[&str]) {
    for package in packages { map.entry((*package).into()).or_default().insert(reason.into()); }
}
fn candidates(plan: &Plan, providers: &[String]) -> BTreeMap<String, BTreeSet<String>> {
    let mut result = BTreeMap::new();
    for step in &plan.steps {
        if step.status != "missing" { continue; }
        if step.source == "atelier-de" { add(&mut result, step.id, &["rust", "base-devel", "pkgconf"]); }
        match step.id {
            "compositor" => add(&mut result, step.id, &["wayland", "libxkbcommon", "libxkbcommon-x11", "mesa", "seatd", "systemd", "libinput", "libdrm"]),
            "shell" => add(&mut result, step.id, &["gtk4", "gtk4-layer-shell"]),
            "settings" => add(&mut result, step.id, &["gtk4"]),
            "notifications" => add(&mut result, step.id, &["dbus"]),
            "mangowm" => add(&mut result, step.id, &["mangowm"]),
            "hyprland" => add(&mut result, step.id, &["hyprland"]),
            "niri" => add(&mut result, step.id, &["niri"]),
            _ => {}
        }
    }
    for provider in providers {
        match provider.as_str() {
            "media" => add(&mut result, "optional media", &["playerctl", "coreutils"]),
            "audio" => add(&mut result, "optional audio", &["libpulse", "coreutils"]),
            "network" => add(&mut result, "optional network", &["networkmanager", "coreutils"]),
            "bluetooth" => add(&mut result, "optional bluetooth", &["bluez-utils", "coreutils"]),
            _ => {}
        }
    }
    result
}
fn missing_from_output(output: Output, packages: &BTreeSet<String>) -> Option<BTreeSet<String>> {
    let stdout = String::from_utf8(output.stdout).ok()?;
    if output.status.success() && stdout.trim().is_empty() { return Some(BTreeSet::new()); }
    if output.status.code() != Some(127) || !output.stderr.is_empty() { return None; }
    let missing: BTreeSet<_> = stdout.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect();
    if missing.is_empty() || !missing.is_subset(packages) { return None; }
    Some(missing)
}
pub fn preview(plan: &Plan, report: &Report, providers: &[String]) -> PackagePreview {
    let mut notes = Vec::new();
    if !arch_family(report) {
        notes.push("No package mapping enabled for the detected distribution. Product inventory remains available.".into());
        return PackagePreview { schema: 1, backend: None, candidates: vec![], notes, apply_supported: false };
    }
    let packages = candidates(plan, providers);
    let names: BTreeSet<_> = packages.keys().cloned().collect();
    let pacman = report.package_managers.iter().find(|tool| tool.name == "pacman").and_then(|tool| tool.path.as_ref());
    let timeout = report.optional_tools.iter().find(|tool| tool.name == "timeout").and_then(|tool| tool.path.as_ref());
    let missing = if names.is_empty() { Some(BTreeSet::new()) }
        else if let (Some(pacman), Some(timeout)) = (pacman, timeout) {
            Command::new(timeout).args(["--kill-after=1s", "5s"]).arg(pacman)
                .arg("-T").args(&names).env("LC_ALL", "C").output().ok()
                .and_then(|output| missing_from_output(output, &names))
        } else { None };
    if missing.is_none() { notes.push("Local dependency query unavailable; package states are unknown.".into()); }
    notes.push("Candidates cover selected missing source-build products and optional providers only; they do not resolve transitive dependencies, prove repository availability or validate found binaries.".into());
    notes.push("This preview performs no installation, database synchronization or system update. The separate install-packages command requires interactive review and sudo.".into());
    if plan.steps.iter().any(|step| step.id == "mango-atelier") {
        notes.push("Mango Atelier rice dependencies and optional app groups are delegated to its existing setup --plan workflow.".into());
    }
    if providers.iter().any(|provider| provider == "audio") {
        notes.push("Audio selects the pactl client only; no audio server replacement is proposed.".into());
    }
    let candidates = packages.into_iter().map(|(package, reasons)| {
        let state = match &missing {
            Some(missing) if missing.contains(&package) => "missing",
            Some(_) => "satisfied",
            None => "unknown",
        };
        Candidate { package, reasons: reasons.into_iter().collect(), state }
    }).collect();
    PackagePreview { schema: 1, backend: Some("pacman"), candidates, notes, apply_supported: false }
}


// The privileged command is constructed solely from catalog-derived package names.
// Never execute a PATH-resolved program as root or refresh without a full upgrade.
const PACMAN: &str = "/usr/bin/pacman";
const SUDO: &str = "/usr/bin/sudo";
const TIMEOUT: &str = "/usr/bin/timeout";

fn trusted_tool(path: &str) -> Result<()> {
    let metadata = fs::metadata(path).with_context(|| format!("inspect {path}"))?;
    if !metadata.is_file() || metadata.uid() != 0 || metadata.permissions().mode() & 0o022 != 0
        || metadata.permissions().mode() & 0o111 == 0 {
        bail!("not a root-owned executable protected from non-root writes: {path}");
    }
    Ok(())
}

fn transaction_args(names: &BTreeSet<String>) -> Result<Vec<String>> {
    if names.is_empty() { bail!("no packages selected"); }
    if names.iter().any(|name| !name.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'_' | b'.'))) {
        bail!("invalid package name");
    }
    let mut args = vec![PACMAN.into(), "-Syu".into(), "--needed".into(), "--".into()];
    args.extend(names.iter().cloned());
    Ok(args)
}

fn query(flag: &str, names: &BTreeSet<String>) -> Result<Output> {
    Command::new(TIMEOUT).args(["--kill-after=1s", "15s", PACMAN, flag])
        .args(names).env("LC_ALL", "C").output().context("query local pacman database")
}

fn checkpoint_root() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute()).map(|p| p.join(".local/state")))
        .context("no absolute state or home directory")?;
    let root = base.join("atelier-manager/package-transactions");
    fs::create_dir_all(&root)?;
    let metadata = fs::symlink_metadata(&root)?;
    if !metadata.is_dir() || metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.permissions().mode() & 0o022 != 0 {
        bail!("package checkpoint root is not an owned safe directory");
    }
    Ok(root)
}

fn snapshot(dir: &Path, name: &str) -> Result<()> {
    let result = query("-Q", &BTreeSet::new())?;
    if !result.status.success() { bail!("could not read installed package inventory"); }
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(dir.join(name))?;
    file.write_all(&result.stdout)?;
    file.sync_all()?;
    Ok(())
}

pub fn install(plan: &Plan, report: &Report) -> Result<()> {
    if !arch_family(report) { bail!("package installation is currently limited to Arch-family systems"); }
    if nix::unistd::Uid::effective().is_root() { bail!("run as your desktop user, not root"); }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("package installation requires an interactive terminal");
    }
    for path in [PACMAN, SUDO, TIMEOUT] { trusted_tool(path)?; }
    let requested: BTreeSet<_> = candidates(plan, &[]).into_keys().collect();
    if requested.is_empty() { println!("No package candidates. Nothing to install."); return Ok(()); }
    let missing = missing_from_output(query("-T", &requested)?, &requested)
        .context("cannot establish missing packages; no installation attempted")?;
    if missing.is_empty() { println!("All candidate dependencies are satisfied. Nothing to install."); return Ok(()); }
    let args = transaction_args(&missing)?;
    println!("Missing catalog package targets:");
    for name in &missing { println!("  {name}"); }
    println!("This uses sudo pacman -Syu --needed and may upgrade the ENTIRE system.");
    println!("Pacman will resolve additional dependencies and present its own transaction review.");
    println!("Repository availability is not guaranteed; no AUR helper or automatic removals are invoked by Atelier.");
    println!("Create a system snapshot separately if you need rollback. Package inventory is diagnostic only.");
    print!("Type 'upgrade and install' to proceed (anything else cancels): "); io::stdout().flush()?;
    let mut answer = String::new(); io::stdin().read_line(&mut answer)?;
    if answer.trim() != "upgrade and install" { println!("Cancelled; no package changes."); return Ok(()); }
    let dir = tempfile::Builder::new().prefix("transaction-").tempdir_in(checkpoint_root()?)?;
    snapshot(dir.path(), "before.txt")?;
    fs::write(dir.path().join("targets.txt"), missing.iter().cloned().collect::<Vec<_>>().join("\n"))?;
    let dir = dir.keep();
    println!("Package inventory checkpoint: {}", dir.display());
    let result = Command::new(SUDO).args(&args).status();
    let after = snapshot(&dir, "after.txt");
    match &result {
        Ok(status) => fs::write(dir.join("outcome.txt"), format!("{status}\n"))?,
        Err(error) => fs::write(dir.join("outcome.txt"), format!("could not start sudo: {error}\n"))?,
    }
    if let Err(error) = after { eprintln!("Post-transaction inventory unavailable: {error}"); }
    let status = result.context("launch package transaction")?;
    if !status.success() { bail!("pacman did not complete successfully; inspect its output and checkpoint at {}. No automatic rollback attempted.", dir.display()); }
    let remaining = missing_from_output(query("-T", &missing)?, &missing)
        .context("pacman exited successfully, but final dependency verification is unavailable")?;
    if !remaining.is_empty() { bail!("packages remain unsatisfied after transaction: {remaining:?}"); }
    println!("Requested dependencies verified. No Atelier binaries, rice settings or sessions were changed by the manager.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::PlanStep;
    use std::os::unix::process::ExitStatusExt;
    fn plan(id: &'static str, source: &'static str) -> Plan {
        Plan { schema: 1, requested: vec![id.into()], steps: vec![PlanStep { id, name: id, source, status: "missing", missing_binaries: vec![] }], warnings: vec![], apply_supported: false }
    }
    #[test]
    fn shell_does_not_pull_native_compositor_or_audio_server() {
        let result = candidates(&plan("shell", "atelier-de"), &["audio".into()]);
        assert!(result.contains_key("gtk4-layer-shell"));
        assert!(result.contains_key("libpulse"));
        assert!(!result.contains_key("libinput"));
        assert!(!result.contains_key("pipewire"));
        assert!(!result.contains_key("pulseaudio"));
    }
    #[test]
    fn transaction_requires_full_upgrade_and_rejects_options() {
        let names = BTreeSet::from(["gtk4".into(), "niri".into()]);
        assert_eq!(transaction_args(&names).unwrap(), vec![PACMAN, "-Syu", "--needed", "--", "gtk4", "niri"]);
        assert!(transaction_args(&BTreeSet::new()).is_err());
        assert!(transaction_args(&BTreeSet::from(["--noconfirm".into()])).is_err());
        assert!(transaction_args(&BTreeSet::from(["foo;bar".into()])).is_err());
    }
    #[test]
    fn local_query_is_fail_closed() {
        let names = BTreeSet::from(["gtk4".into()]);
        let output = |code, stdout: &str| Output { status: std::process::ExitStatus::from_raw(code << 8), stdout: stdout.as_bytes().to_vec(), stderr: vec![] };
        assert!(missing_from_output(output(0, ""), &names).unwrap().is_empty());
        assert_eq!(missing_from_output(output(127, "gtk4\n"), &names), Some(names.clone()));
        assert!(missing_from_output(output(127, "unknown\n"), &names).is_none());
        assert!(missing_from_output(output(1, ""), &names).is_none());
    }
}
