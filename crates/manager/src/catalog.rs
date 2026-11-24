//! Product graph shared by future installer and graphical manager.
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::BTreeSet;
use crate::Report;

#[derive(Debug, Serialize)]
pub struct Product {
    pub id: &'static str,
    pub name: &'static str,
    pub source: &'static str,
    pub binaries: &'static [&'static str],
    pub requires: &'static [&'static str],
    pub note: &'static str,
}

pub const PRODUCTS: &[Product] = &[
    Product { id: "compositor", name: "Atelier Compositor", source: "atelier-de", binaries: &["atelier-compositor"], requires: &[], note: "Experimental native hardware session; nested testing supported." },
    Product { id: "shell", name: "Atelier Shell", source: "atelier-de", binaries: &["atelier-shell"], requires: &[], note: "Independent launcher; panel support depends on build and Wayland protocols." },
    Product { id: "session", name: "Atelier Session", source: "atelier-de", binaries: &["atelier-session"], requires: &["compositor"], note: "Can start the default shell, an external shell, or no shell." },
    Product { id: "settings", name: "Atelier Settings", source: "atelier-de", binaries: &["atelier-settings"], requires: &[], note: "Standalone desktop settings; system providers are optional." },
    Product { id: "services", name: "Atelier Services", source: "atelier-de", binaries: &["atelier-services"], requires: &[], note: "Independent power, media, audio, network and Bluetooth CLI." },
    Product { id: "notifications", name: "Atelier Notifications", source: "atelier-de", binaries: &["atelier-notifications"], requires: &[], note: "Explicit opt-in inbox; installing does not enable daemon ownership." },
    Product { id: "manager", name: "Atelier Manager", source: "atelier-de", binaries: &["atelier-manager"], requires: &[], note: "Interactive setup, release and deployment lifecycle management." },
    Product { id: "mango-atelier", name: "Mango Atelier", source: "mango-atelier", binaries: &["atelier", "atelier-workspace"], requires: &[], note: "Separate rice distribution; choose a supported compositor and use its existing installer." },
    Product { id: "mangowm", name: "MangoWM", source: "external", binaries: &["mango"], requires: &[], note: "Third-party compositor, independent from Atelier-DE." },
    Product { id: "hyprland", name: "Hyprland", source: "external", binaries: &["Hyprland"], requires: &[], note: "Third-party compositor; runtime support must be validated." },
    Product { id: "niri", name: "Niri", source: "external", binaries: &["niri"], requires: &[], note: "Third-party compositor; runtime support must be validated." },
    Product { id: "desktop", name: "Atelier Desktop", source: "aggregate", binaries: &[], requires: &["compositor", "shell", "session", "settings", "services", "notifications", "manager"], note: "Native desktop product bundle, not a released distribution or ISO." },
];

fn lookup(id: &str) -> Option<&'static Product> { PRODUCTS.iter().find(|product| product.id == id) }
fn visit(id: &str, visiting: &mut BTreeSet<&'static str>, done: &mut BTreeSet<&'static str>, result: &mut Vec<&'static Product>) -> Result<()> {
    let product = lookup(id).with_context(|| format!("unknown product: {id}; use atelier-manager catalog"))?;
    if done.contains(product.id) { return Ok(()); }
    if !visiting.insert(product.id) { bail!("product dependency cycle at {}", product.id); }
    for dependency in product.requires { visit(dependency, visiting, done, result)?; }
    visiting.remove(product.id); done.insert(product.id); result.push(product);
    Ok(())
}
pub fn resolve(ids: &[String]) -> Result<Vec<&'static Product>> {
    if ids.is_empty() { bail!("select at least one product"); }
    let mut result = Vec::new(); let mut visiting = BTreeSet::new(); let mut done = BTreeSet::new();
    for id in ids { visit(id, &mut visiting, &mut done, &mut result)?; }
    Ok(result)
}

#[derive(Debug, Serialize)]
pub struct PlanStep { pub id: &'static str, pub name: &'static str, pub source: &'static str, pub status: &'static str, pub missing_binaries: Vec<&'static str> }
#[derive(Debug, Serialize)]
pub struct Plan {
    pub schema: u32,
    pub requested: Vec<String>,
    pub steps: Vec<PlanStep>,
    pub warnings: Vec<String>,
    pub apply_supported: bool,
}
pub fn plan(ids: &[String], report: &Report) -> Result<Plan> {
    let products = resolve(ids)?;
    let found = |name: &&str| report.products.iter().chain(report.compositors.iter()).any(|entry| entry.name == *name && entry.path.is_some());
    let steps = products.iter().map(|product| {
        let missing_binaries: Vec<_> = product.binaries.iter().filter(|name| !found(name)).copied().collect();
        let status = if product.binaries.is_empty() { "aggregate" } else if missing_binaries.is_empty() { "found" } else { "missing" };
        PlanStep { id: product.id, name: product.name, source: product.source, status, missing_binaries }
    }).collect();
    let mut warnings = vec!["This plan is preview-only. Inventory does not verify runtime compatibility or package availability; installation requires a separate confirmed operation.".into()];
    if products.iter().any(|p| p.id == "mango-atelier") {
        warnings.push("Mango Atelier requires an explicitly chosen supported compositor and rice configuration; no third-party compositor is selected automatically.".into());
    }
    if products.iter().any(|p| p.id == "compositor") {
        warnings.push("Native DRM login remains experimental pending real hardware acceptance. Keep your existing desktop.".into());
    }
    if products.iter().any(|p| p.id == "notifications") {
        warnings.push("Notification daemon will not be started or replace an existing owner.".into());
    }
    Ok(Plan { schema: 1, requested: ids.to_vec(), steps, warnings, apply_supported: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_resolves_each_native_component_once_without_mango() {
        let resolved = resolve(&["desktop".into(), "shell".into(), "session".into()]).unwrap();
        let ids: Vec<_> = resolved.iter().map(|product| product.id).collect();
        assert_eq!(ids.len(), 8);
        assert!(!ids.contains(&"mango-atelier"));
        assert!(ids.iter().position(|id| *id == "compositor").unwrap() < ids.iter().position(|id| *id == "session").unwrap());
    }
    #[test]
    fn standalone_shell_and_external_compositors_do_not_pull_native_desktop() {
        assert_eq!(resolve(&["shell".into()]).unwrap().len(), 1);
        assert_eq!(resolve(&["hyprland".into(), "mango-atelier".into()]).unwrap().len(), 2);
        assert!(resolve(&["unknown".into()]).is_err());
        assert!(resolve(&[]).is_err());
    }
}
