//! Guided read-only setup. Input and output are injectable for regression tests.
use anyhow::{bail, Result};
use serde::Serialize;
use std::io::{BufRead, Write};
use crate::{catalog::{self, Plan}, Report};

pub(super) const EXTERNAL: &[&str] = &["mangowm", "hyprland", "niri"];
pub(super) const PROVIDERS: &[&str] = &["media", "audio", "network", "bluetooth"];

#[derive(Debug, Serialize)]
pub struct RiceChoice { pub compositor: String, pub rice: String }
#[derive(Debug, Serialize)]
pub struct Preview {
    schema: u32,
    mode: String,
    plan: Plan,
    packages: crate::packages::PackagePreview,
    rices: Vec<RiceChoice>,
    native_shell: Option<String>,
    optional_providers: Vec<String>,
    first_session: Option<String>,
    apply_supported: bool,
}

impl Preview {
    pub(super) fn requested_products(&self) -> &[String] { &self.plan.requested }
    pub(super) fn install_packages(&self, report: &Report) -> Result<()> {
        crate::packages::install(&self.plan, report, &self.optional_providers)
    }
}

fn ask(input: &mut impl BufRead, out: &mut impl Write, prompt: &str) -> Result<Option<String>> {
    write!(out, "{prompt} ")?; out.flush()?;
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 { return Ok(None); }
    let value = line.trim();
    if value.eq_ignore_ascii_case("q") || value.eq_ignore_ascii_case("quit") { return Ok(None); }
    Ok(Some(value.to_owned()))
}
fn selections(value: &str) -> Vec<String> {
    let mut result = Vec::new();
    for id in value.split(',').map(str::trim).filter(|id| !id.is_empty()) {
        if !result.iter().any(|existing| existing == id) { result.push(id.to_owned()); }
    }
    result
}
fn select_from(input: &mut impl BufRead, out: &mut impl Write, prompt: &str, allowed: &[&str], default: &str) -> Result<Option<Vec<String>>> {
    loop {
        let Some(value) = ask(input, out, prompt)? else { return Ok(None); };
        let ids = selections(if value.is_empty() { default } else { &value });
        if !ids.is_empty() && ids.iter().all(|id| allowed.contains(&id.as_str())) { return Ok(Some(ids)); }
        writeln!(out, "Choose comma-separated IDs from the listed options, or q to cancel.")?;
    }
}
fn valid_rice(id: &str) -> bool {
    (2..=48).contains(&id.len()) && id.as_bytes()[0].is_ascii_lowercase()
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
}

pub fn run(input: &mut impl BufRead, out: &mut impl Write, report: &Report) -> Result<Option<Preview>> {
    writeln!(out, "Atelier Setup — preview only\nType q at any prompt to cancel.")?;
    writeln!(out, "1. System: {} / {} / {}", report.distribution.id.as_deref().unwrap_or("unknown"),
        report.architecture, report.desktop.as_deref().unwrap_or("unknown desktop"))?;
    writeln!(out, "2. Choose setup mode\n  1 Express: complete experimental native desktop\n  2 Custom: select individual products or external compositors\n  3 Minimal: standalone shell and settings")?;
    let mode = loop {
        let Some(value) = ask(input, out, "Mode [1/2/3; default 2]:")? else { return Ok(None); };
        match value.as_str() {
            "1" => break "express",
            "" | "2" => break "custom",
            "3" => break "minimal",
            _ => writeln!(out, "Choose 1, 2 or 3.")?,
        }
    };
    let mut ids = match mode {
        "express" => vec!["desktop".into()],
        "minimal" => vec!["shell".into(), "settings".into()],
        _ => {
            writeln!(out, "3. Available products (found status is checked at review):")?;
            for product in catalog::PRODUCTS { writeln!(out, "  {} — {}", product.id, product.name)?; }
            let allowed: Vec<_> = catalog::PRODUCTS.iter().map(|product| product.id).collect();
            let Some(ids) = select_from(input, out, "Product IDs, comma-separated [shell,settings]:", &allowed, "shell,settings")? else { return Ok(None); };
            ids
        }
    };

    let mut rices = Vec::new();
    if ids.iter().any(|id| id == "mango-atelier") {
        let selected: Vec<_> = ids.iter().filter(|id| EXTERNAL.contains(&id.as_str())).cloned().collect();
        let selected = if selected.is_empty() {
            writeln!(out, "Mango Atelier needs an explicitly selected compositor: mangowm, hyprland or niri.")?;
            let Some(selected) = select_from(input, out, "Compositor IDs [mangowm]:", EXTERNAL, "mangowm")? else { return Ok(None); };
            ids.extend(selected.iter().cloned()); selected
        } else { selected };
        for compositor in selected {
            writeln!(out, "Rice for {compositor}: keep, atelier (Classic), serpantinum (must already be installed){}", if compositor == "niri" { "" } else { ", sekaipop (experimental)" })?;
            writeln!(out, "Custom registered rice IDs are allowed but must be verified by Mango Atelier before apply.")?;
            let rice = loop {
                let Some(value) = ask(input, out, "Rice ID [keep]:")? else { return Ok(None); };
                let value = if value.is_empty() { "keep".into() } else { value };
                if value == "sekaipop" && compositor == "niri" { writeln!(out, "SekaiPop is not currently supported on Niri.")?; continue; }
                if valid_rice(&value) { break value; }
                writeln!(out, "Enter a valid rice ID.")?;
            };
            rices.push(RiceChoice { compositor, rice });
        }
    }

    let resolved = catalog::resolve(&ids)?;
    let has_session = resolved.iter().any(|product| product.id == "session");
    let native_shell = if has_session {
        writeln!(out, "Native session shell: default, none, or an existing named profile.")?;
        loop {
            let Some(value) = ask(input, out, "Shell choice [default], none, or profile:NAME:")? else { return Ok(None); };
            let value = if value.is_empty() { "default".into() } else { value };
            let valid_profile = value.strip_prefix("profile:").is_some_and(|name| !name.is_empty() && name.len() <= 48 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
            if matches!(value.as_str(), "default" | "none") || valid_profile { break Some(value); }
            writeln!(out, "Use default, none, or profile:NAME.")?;
        }
    } else { None };

    writeln!(out, "4. Optional providers: media, audio, network, bluetooth (or none).")?;
    let optional_providers = loop {
        let Some(value) = ask(input, out, "Provider IDs, comma-separated [none]:")? else { return Ok(None); };
        if value.is_empty() || value == "none" { break Vec::new(); }
        let providers = selections(&value);
        if !providers.is_empty() && providers.iter().all(|id| PROVIDERS.contains(&id.as_str())) { break providers; }
        writeln!(out, "Choose supported provider IDs or none.")?;
    };
    let mut sessions: Vec<_> = ids.iter().filter(|id| EXTERNAL.contains(&id.as_str())).map(String::as_str).collect();
    if has_session { sessions.push("atelier"); }
    let first_session = if sessions.is_empty() { None } else {
        sessions.push("keep");
        writeln!(out, "5. First session preference: {}", sessions.join(", "))?;
        loop {
            let Some(value) = ask(input, out, "First session [keep current]:")? else { return Ok(None); };
            let value = if value.is_empty() { "keep".into() } else { value };
            if sessions.contains(&value.as_str()) { break Some(value); }
            writeln!(out, "Choose a listed session or keep.")?;
        }
    };

    let preview = build_preview(Choices { mode: mode.into(), ids, rices, native_shell, optional_providers, first_session }, report)?;
    preview.write_review(out)?;
    let Some(answer) = ask(input, out, "Accept this preview? [y/N]:")? else { return Ok(None); };
    if !matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes") { return Ok(None); }
    Ok(Some(preview))
}

/// Shared setup choices; frontends supply intent, not precomputed dependencies.
pub(super) struct Choices {
    pub mode: String,
    pub ids: Vec<String>,
    pub rices: Vec<RiceChoice>,
    pub native_shell: Option<String>,
    pub optional_providers: Vec<String>,
    pub first_session: Option<String>,
}

pub(super) fn build_preview(choices: Choices, report: &Report) -> Result<Preview> {
    let Choices { mode, ids, rices, native_shell, optional_providers, first_session } = choices;
    if !matches!(mode.as_str(), "express" | "custom" | "minimal") { bail!("invalid setup mode"); }
    let resolved = catalog::resolve(&ids)?;
    let external: std::collections::BTreeSet<_> = ids.iter().filter(|id| EXTERNAL.contains(&id.as_str())).map(String::as_str).collect();
    let mango = ids.iter().any(|id| id == "mango-atelier");
    if mango && external.is_empty() { bail!("Mango Atelier requires at least one explicitly selected external compositor"); }
    let rice_targets: std::collections::BTreeSet<_> = rices.iter().map(|rice| rice.compositor.as_str()).collect();
    if rice_targets.len() != rices.len() || (mango && rice_targets != external) || (!mango && !rices.is_empty()) {
        bail!("choose exactly one rice for each selected Mango Atelier compositor");
    }
    for rice in &rices {
        if !valid_rice(&rice.rice) || (rice.compositor == "niri" && rice.rice == "sekaipop") {
            bail!("invalid or unsupported rice for {}", rice.compositor);
        }
    }
    let has_session = resolved.iter().any(|p| p.id == "session");
    match (&native_shell, has_session) {
        (Some(value), true) if matches!(value.as_str(), "default" | "none")
            || value.strip_prefix("profile:").is_some_and(|name| !name.is_empty() && name.len() <= 48 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')) => {},
        (None, false) => {},
        _ => bail!("choose default, none, or profile:NAME for a native session"),
    }
    if optional_providers.iter().any(|id| !PROVIDERS.contains(&id.as_str())) { bail!("unsupported optional provider"); }
    let has_sessions = has_session || !external.is_empty();
    match first_session.as_deref() {
        Some("keep") if has_sessions => {},
        Some("atelier") if has_session => {},
        Some(id) if external.contains(id) => {},
        None if !has_sessions => {},
        _ => bail!("first session must be selected or keep"),
    }
    let plan = catalog::plan(&ids, report)?;
    let packages = crate::packages::preview(&plan, report, &optional_providers);
    Ok(Preview { schema: 1, mode, plan, packages, rices, native_shell, optional_providers, first_session, apply_supported: false })
}

impl Preview {
    pub(super) fn write_review(&self, out: &mut impl Write) -> Result<()> {
        writeln!(out, "\n6. Review — no changes will be applied")?;
        for step in &self.plan.steps { writeln!(out, "  {}: {} · {}", step.name, step.status, step.source)?; }
        for rice in &self.rices { writeln!(out, "  Rice for {}: {}", rice.compositor, rice.rice)?; }
        if let Some(shell) = &self.native_shell { writeln!(out, "  Native shell: {shell}")?; }
        writeln!(out, "  Optional providers: {}", if self.optional_providers.is_empty() { "none".into() } else { self.optional_providers.join(", ") })?;
        if let Some(session) = &self.first_session { writeln!(out, "  First session: {session}")?; }
        for warning in &self.plan.warnings { writeln!(out, "  Note: {warning}")?; }
        for candidate in &self.packages.candidates { writeln!(out, "  Package {}: {} ({})", candidate.package, candidate.state, candidate.reasons.join(", "))?; }
        for note in &self.packages.notes { writeln!(out, "  Package note: {note}")?; }
        writeln!(out, "  Package changes: none\n  User files/backups: none\n  sudo: not used\n  Recovery: no transaction performed")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn preview(input: &str) -> (Option<Preview>, String) {
        let mut out = Vec::new();
        let report = Report { schema: 1, distribution: Default::default(), distribution_error: None, architecture: "test",
            session_type: None, desktop: None, package_managers: vec![], products: vec![], compositors: vec![], optional_tools: vec![] };
        let result = run(&mut Cursor::new(input), &mut out, &report).unwrap();
        (result, String::from_utf8(out).unwrap())
    }
    #[test]
    fn minimal_setup_never_selects_compositor() {
        let (result, output) = preview("3\nnone\ny\n");
        let result = result.unwrap();
        assert_eq!(result.plan.steps.len(), 2);
        assert_eq!(result.requested_products(), &["shell".to_owned(), "settings".to_owned()]);
        assert!(!result.apply_supported);
        assert!(result.first_session.is_none());
        assert!(output.contains("sudo: not used"));
    }
    #[test]
    fn mango_multiple_compositors_and_rices_are_independent() {
        let (result, _) = preview("2\nmango-atelier\nhyprland,niri\nsekaipop\nsekaipop\natelier\nmedia,audio\nniri\ny\n");
        let result = result.unwrap();
        assert_eq!(result.rices[0].rice, "sekaipop");
        assert_eq!(result.rices[1].rice, "atelier");
        assert_eq!(result.first_session.as_deref(), Some("niri"));
        assert_eq!(result.optional_providers, vec!["media", "audio"]);
    }
    #[test]
    fn cancellation_and_eof_never_complete_preview() {
        assert!(preview("q\n").0.is_none());
        assert!(preview("1\n").0.is_none());
        assert!(preview("3\nnone\nn\n").0.is_none());
    }
}
