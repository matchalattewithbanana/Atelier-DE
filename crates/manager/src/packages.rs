//! Conservative Arch-family package candidates and local dependency checks.
use crate::{catalog::Plan, Report};
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
    notes.push("A future package transaction requires explicit review and may need sudo. No installation, database synchronization or system update is performed here.".into());
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
    fn local_query_is_fail_closed() {
        let names = BTreeSet::from(["gtk4".into()]);
        let output = |code, stdout: &str| Output { status: std::process::ExitStatus::from_raw(code << 8), stdout: stdout.as_bytes().to_vec(), stderr: vec![] };
        assert!(missing_from_output(output(0, ""), &names).unwrap().is_empty());
        assert_eq!(missing_from_output(output(127, "gtk4\n"), &names), Some(names.clone()));
        assert!(missing_from_output(output(127, "unknown\n"), &names).is_none());
        assert!(missing_from_output(output(1, ""), &names).is_none());
    }
}
