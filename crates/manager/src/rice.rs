//! Optional trusted-checkout integration with Mango Atelier's existing Rice Manager.
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::{fs, io::{self, Read}, os::unix::fs::{MetadataExt, PermissionsExt}, path::{Path, PathBuf}, process::{Command, Stdio}, thread};

const MAX_OUTPUT: u64 = 2 * 1024 * 1024;
const COMPOSITORS: &[&str] = &["mango", "hyprland", "niri"];
const MARKERS: &[&str] = &["MANGO_INSTANCE_SIGNATURE", "HYPRLAND_INSTANCE_SIGNATURE", "NIRI_SOCKET"];
#[derive(Clone, Copy)]
pub(super) enum Action { Local, Discover, Snapshots, Prepare, Select }
pub(super) struct Item { pub title: String, pub status: String, pub detail: String }

#[derive(Deserialize)]
struct Catalog { compositor: String, rices: Vec<Profile>, warnings: Vec<String> }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    id: String, name: String, summary: String, ready: bool, selected: bool, supported: bool,
    compositors: Vec<String>, install_method: String, missing_tools: Vec<String>,
}
#[derive(Deserialize)]
struct Community { status: String, rices: Vec<Listing> }
#[derive(Deserialize)]
struct Listing { name: String, author: String, source: String, compatibility: String }
#[derive(Deserialize)]
struct Snapshot { id: String, rice: String, name: String, files: u64 }

fn valid_id(id: &str) -> bool {
    (2..=48).contains(&id.len()) && id.as_bytes()[0].is_ascii_lowercase()
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
}
fn trusted_tool(path: &str) -> Result<()> {
    let m = fs::metadata(path).with_context(|| format!("required system tool unavailable: {path}"))?;
    if !m.is_file() || m.uid() != 0 || m.permissions().mode() & 0o022 != 0 || m.permissions().mode() & 0o111 == 0 {
        bail!("unsafe system tool: {path}");
    }
    Ok(())
}
fn read_limited(stream: impl Read, limit: u64) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    stream.take(limit + 1).read_to_end(&mut data)?;
    Ok(data)
}
fn capture(mut command: Command) -> Result<Vec<u8>> {
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let stdout = child.stdout.take().context("missing stdout")?;
    let stderr = child.stderr.take().context("missing stderr")?;
    let worker = thread::spawn(move || read_limited(stderr, 16 * 1024));
    let out = read_limited(stdout, MAX_OUTPUT);
    let status = child.wait()?;
    let err = worker.join().map_err(|_| anyhow::anyhow!("stderr reader failed"))??;
    let out = out?;
    if out.len() as u64 > MAX_OUTPUT { bail!("rice response exceeds size limit"); }
    if !status.success() {
        bail!("Mango Atelier returned {status}: {}", String::from_utf8_lossy(&err));
    }
    Ok(out)
}

struct Checkout { root: PathBuf }
impl Checkout {
    fn open(path: &Path) -> Result<Self> {
        if nix::unistd::Uid::effective().is_root() { bail!("run as your desktop user, not root"); }
        if !path.is_absolute() { bail!("enter the absolute path to a trusted Mango Atelier checkout"); }
        let root = path.canonicalize()?;
        for file in ["setup.sh", "scripts/rice_manager.py"] {
            if !fs::symlink_metadata(root.join(file))?.is_file() { bail!("checkout lacks regular {file}"); }
        }
        for path in ["/usr/bin/bash", "/usr/bin/python3", "/usr/bin/timeout"] { trusted_tool(path)?; }
        let checkout = Self { root };
        let mut cmd = checkout.command("25s");
        cmd.args(["/usr/bin/bash", "--noprofile", "--norc"]).arg(checkout.root.join("setup.sh")).arg("--manager-api");
        if capture(cmd)? != format!("{}\n", crate::mango::API).as_bytes() {
            bail!("unsupported Mango Atelier checkout; use the trusted integration branch");
        }
        Ok(checkout)
    }
    fn command(&self, limit: &str) -> Command {
        let mut cmd = Command::new("/usr/bin/timeout");
        cmd.args(["--kill-after=2s", limit]).current_dir(&self.root)
            .env_remove("BASH_ENV").env_remove("ENV").env_remove("ATELIER_ALLOW_ROOT_FOR_TESTS")
            .env_remove("PYTHONPATH").env_remove("PYTHONHOME");
        cmd
    }
    fn run(&self, target: &str, args: &[&str], limit: &str) -> Result<Vec<u8>> {
        if !COMPOSITORS.contains(&target) { bail!("unsupported compositor"); }
        let mut cmd = self.command(limit);
        // Catalog/next-login operations are explicitly offline-targeted;
        // never inherit an unrelated live compositor's detection markers.
        for marker in MARKERS { cmd.env_remove(marker); }
        cmd.env("ATELIER_COMPOSITOR", target)
            .args(["/usr/bin/python3", "-E", "-s", "-B"]).arg(self.root.join("scripts/rice_manager.py")).args(args);
        capture(cmd)
    }
    fn catalog(&self, target: &str) -> Result<Catalog> {
        parse_catalog(&self.run(target, &["list", "--json"], "25s")?, target)
    }
}
fn parse_catalog(bytes: &[u8], target: &str) -> Result<Catalog> {
    let data: Catalog = serde_json::from_slice(bytes).context("unsupported rice catalog response")?;
    if data.compositor != target || data.rices.len() > 1024 || data.rices.iter().any(|r| !valid_id(&r.id)) {
        bail!("invalid or mismatched rice catalog");
    }
    let mut ids = std::collections::BTreeSet::new();
    if data.rices.iter().any(|r| !ids.insert(&r.id)) { bail!("duplicate rice ID"); }
    Ok(data)
}
fn mutation_args(action: Action, target: &str, id: &str) -> Result<Vec<String>> {
    if !valid_id(id) || !COMPOSITORS.contains(&target) { bail!("invalid rice or compositor ID"); }
    let mut args = vec![match action {
        Action::Prepare if id == "sekaipop" => "install",
        Action::Prepare => bail!("only the curated SekaiPop preparation is integrated"),
        Action::Select => "use",
        _ => bail!("not a mutation"),
    }.into(), id.into(), "--compositor".into(), target.into()];
    if matches!(action, Action::Select) { args.push("--next-login".into()); }
    Ok(args)
}
pub(super) fn execute(path: &Path, target: &str, action: Action, id: &str) -> Result<Vec<Item>> {
    if !COMPOSITORS.contains(&target) { bail!("select a supported target"); }
    let checkout = Checkout::open(path)?;
    let mut items = Vec::new();
    match action {
        Action::Local => {
            let catalog = checkout.catalog(target)?;
            for r in catalog.rices {
                let status = if !r.supported { "Unsupported" } else if r.selected { if r.ready { "Selected · ready" } else { "Selected · needs setup" } } else if r.ready { "Ready" } else { "Needs setup" };
                items.push(Item { title: r.name, status: status.into(), detail: format!("ID: {} · {}\n{}\nCompatible: {}\nMissing tools: {}", r.id, r.install_method, r.summary, r.compositors.join(", "), r.missing_tools.join(", ")) });
            }
            for warning in catalog.warnings { items.push(Item { title: "Registry warning".into(), status: "Ignored invalid rice".into(), detail: warning }); }
        }
        Action::Discover => {
            let data: Community = serde_json::from_slice(&checkout.run(target, &["discover", "--refresh", "--json"], "30s")?)?;
            items.push(Item { title: "Rice Cooker".into(), status: data.status, detail: "Community metadata only; no community install commands are accepted. Hyprland listings are not proof of compatibility.".into() });
            for entry in data.rices { items.push(Item { title: entry.name, status: entry.compatibility, detail: format!("By {}\n{}", entry.author, entry.source) }); }
        }
        Action::Snapshots => {
            let data: Vec<Snapshot> = serde_json::from_slice(&checkout.run(target, &["snapshot", "list", "--json"], "30s")?)?;
            for entry in data { items.push(Item { title: entry.name, status: format!("{} · {} files", entry.rice, entry.files), detail: entry.id }); }
        }
        Action::Prepare | Action::Select => {
            let args = mutation_args(action, target, id)?;
            let catalog = checkout.catalog(target)?;
            let profile = catalog.rices.iter().find(|r| r.id == id).context("rice not found in local catalog")?;
            if !profile.supported { bail!("rice is unsupported on the selected compositor"); }
            if matches!(action, Action::Prepare) && profile.install_method != "managed" { bail!("rice is not a managed profile"); }
            if matches!(action, Action::Select) && !profile.ready { bail!("prepare and validate the rice before selecting it"); }
            let args: Vec<_> = args.iter().map(String::as_str).collect();
            let output = checkout.run(target, &args, "180s")?;
            items.push(Item { title: "Mango Atelier operation completed".into(), status: "Review delegated output".into(), detail: String::from_utf8_lossy(&output).into_owned() });
        }
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actions_are_closed_and_selection_is_next_login_only() {
        assert_eq!(mutation_args(Action::Select,"niri","atelier").unwrap(), vec!["use","atelier","--compositor","niri","--next-login"]);
        assert!(mutation_args(Action::Prepare,"mango","community-rice").is_err());
        assert!(mutation_args(Action::Select,"mango","--yes").is_err());
        assert!(mutation_args(Action::Select,"unknown","atelier").is_err());
    }
    #[test]
    fn catalog_checks_target_and_duplicate_ids() {
        let data = br#"{"compositor":"mango","rices":[{"id":"atelier","name":"Classic","summary":"","ready":true,"selected":true,"supported":true,"compositors":["mango"],"installMethod":"included","missingTools":[]}],"warnings":[]}"#;
        assert!(parse_catalog(data,"mango").is_ok());
        assert!(parse_catalog(data,"niri").is_err());
        let mut value: serde_json::Value = serde_json::from_slice(data).unwrap();
        let row = value["rices"][0].clone();
        value["rices"].as_array_mut().unwrap().push(row);
        assert!(parse_catalog(&serde_json::to_vec(&value).unwrap(),"mango").is_err());
    }
}
