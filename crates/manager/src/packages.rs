//! Conservative Arch-family package candidates and local dependency checks.
use crate::{catalog::Plan, Report};
use anyhow::{bail, Context, Result};
use std::{fs, io::{self, IsTerminal, Read, Write}, os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt}, path::{Path, PathBuf}};
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
            id => if let Some(package)=crate::customization::provider_package(id){add(&mut result,id,&[package]);}
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
fn cursor_notes(providers:&[String],notes:&mut Vec<String>){
    for (id,package,url) in [
        ("bibata-cursors","bibata-cursor-theme-bin","https://github.com/ful1e5/Bibata_Cursor/releases"),
        ("googledot-cursors","googledot-cursor-theme","https://github.com/ful1e5/Google_Cursor/releases"),
    ]{
        if providers.iter().any(|p|p==id){
            notes.push(format!("{id}: optional AUR package {package}, or upstream XCursor assets at {url}. Not part of the privileged pacman transaction. Use the repository scripts/install-cursors.py user-local installer or install independently; then select the theme in Settings."));
        }
    }
}
pub fn preview(plan: &Plan, report: &Report, providers: &[String]) -> PackagePreview {
    let mut notes = Vec::new();
    cursor_notes(providers,&mut notes);
    if providers.iter().any(|id|crate::customization::provider_package(id).is_some()){
        notes.push("Customization packages are optional installations only. Atelier does not enable display managers, configure PAM, launch lockers or change desktop themes through this package transaction.".into());
    }
    if !arch_family(report) {
        notes.push("No package mapping enabled for the detected distribution. Product inventory remains available.".into());
        return PackagePreview { schema: 1, backend: None, candidates: vec![], notes, apply_supported: false };
    }
    let packages = candidates(plan, providers);
    let names: BTreeSet<_> = packages.keys().cloned().collect();
    let missing = if names.is_empty() { Some(BTreeSet::new()) }
        else if trusted_tool(PACMAN).is_ok() && trusted_tool(TIMEOUT).is_ok() {
            Command::new(TIMEOUT).args(["--kill-after=1s", "5s"]).arg(PACMAN)
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
    Ok(root)
}

fn checkpoint_directory(root: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() || metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.permissions().mode() & 0o022 != 0 {
        bail!("package checkpoint root is not an owned safe directory");
    }
    Ok(())
}

fn snapshot(dir: &Path, name: &str) -> Result<()> {
    let result = query("-Q", &BTreeSet::new())?;
    if !result.status.success() { bail!("could not read installed package inventory"); }
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(dir.join(name))?;
    file.write_all(&result.stdout)?;
    file.sync_all()?;
    Ok(())
}

pub(super) fn parse_install_args(args: &[String]) -> Result<(Vec<String>, Vec<String>)> {
    let mut products = Vec::new();
    let mut providers = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if arg == "--provider" {
            let id = rest.next().context("missing provider ID")?;
            if !crate::wizard::PROVIDERS.contains(&id.as_str()) { bail!("unsupported provider: {id}"); }
            if !providers.contains(id) { providers.push(id.clone()); }
        } else if arg.starts_with('-') { bail!("unsupported install-packages option: {arg}"); }
        else { products.push(arg.clone()); }
    }
    crate::catalog::resolve(&products)?;
    Ok((products, providers))
}

/// Launch the existing interactive CLI in a real terminal. It recomputes
/// dependencies and owns both typed approval and the sudo/pacman prompts.
#[cfg(feature = "gui")]
pub(super) fn terminal_install(products: Vec<String>, providers: Vec<String>) -> Result<()> {
    if nix::unistd::Uid::effective().is_root() { bail!("run as your desktop user, not root"); }
    if !arch_family(&crate::report()) { bail!("package installation is currently limited to Arch-family systems"); }
    let mut args = products;
    for provider in providers { args.extend(["--provider".into(), provider]); }
    parse_install_args(&args)?;
    let manager = std::env::current_exe()?.canonicalize()?;
    let mut terminals: Vec<(&str, &[&str])> = Vec::new();
    if std::env::var_os("WAYLAND_DISPLAY").is_some() { terminals.push(("/usr/bin/foot", &["--hold"])); }
    terminals.extend([("/usr/bin/konsole", &["--hold", "-e"][..]), ("/usr/bin/kitty", &["--hold"][..]), ("/usr/bin/xterm", &["-hold", "-e"][..])]);
    let (terminal, prefix) = terminals.into_iter().find(|(path, _)| trusted_tool(path).is_ok())
        .context("no supported system terminal found (foot, Konsole, kitty, xterm); use install-packages from an existing terminal")?;
    let status = Command::new(terminal).args(prefix).arg(manager).arg("install-packages").args(&args).status()?;
    if !status.success() { bail!("terminal exited with {status}; inspect package history and terminal output"); }
    Ok(())
}

pub fn install(plan: &Plan, report: &Report, providers: &[String]) -> Result<()> {
    if providers.iter().any(|id| !crate::wizard::PROVIDERS.contains(&id.as_str())) { bail!("unsupported provider"); }
    if !arch_family(report) { bail!("package installation is currently limited to Arch-family systems"); }
    if nix::unistd::Uid::effective().is_root() { bail!("run as your desktop user, not root"); }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("package installation requires an interactive terminal");
    }
    for path in [PACMAN, SUDO, TIMEOUT] { trusted_tool(path)?; }
    let mut cursor_instructions=Vec::new();cursor_notes(providers,&mut cursor_instructions);
    for note in cursor_instructions{println!("{note}");}
    let requested: BTreeSet<_> = candidates(plan, providers).into_keys().collect();
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
    let root = checkpoint_root()?;
    fs::create_dir_all(&root)?;
    checkpoint_directory(&root)?;
    let dir = tempfile::Builder::new().prefix("transaction-").tempdir_in(&root)?;
    snapshot(dir.path(), "before.txt")?;
    let mut targets = fs::OpenOptions::new().write(true).create_new(true).open(dir.path().join("targets.txt"))?;
    writeln!(targets, "{}", missing.iter().cloned().collect::<Vec<_>>().join("\n"))?;
    targets.sync_all()?;
    fs::File::open(dir.path())?.sync_all()?;
    let dir = dir.keep();
    fs::File::open(dir.parent().context("checkpoint has no parent")?)?.sync_all()?;
    println!("Package inventory checkpoint: {}", dir.display());
    let result = Command::new(SUDO).args(&args).status();
    let after = snapshot(&dir, "after.txt");
    let outcome = match &result {
        Ok(status) => format!("{status}\n"),
        Err(error) => format!("could not start sudo: {error}\n"),
    };
    record(&dir, "outcome.txt", &outcome)?;
    if let Err(error) = after { eprintln!("Post-transaction inventory unavailable: {error}"); }
    let status = match result {
        Ok(status) => status,
        Err(error) => {
            record(&dir, "verification.txt", "launch-failed\n")?;
            return Err(error).context("launch package transaction");
        }
    };
    if !status.success() {
        record(&dir, "verification.txt", "transaction-failed\n")?;
        bail!("pacman did not complete successfully; inspect its output and checkpoint at {}. No automatic rollback attempted.", dir.display());
    }
    let remaining = match query("-T", &requested).ok().and_then(|out| missing_from_output(out, &requested)) {
        Some(remaining) => remaining,
        None => {
            record(&dir, "verification.txt", "verification-unavailable\n")?;
            bail!("pacman exited successfully, but final dependency verification is unavailable");
        }
    };
    if !remaining.is_empty() {
        record(&dir, "verification.txt", "dependencies-unsatisfied\n")?;
        bail!("packages remain unsatisfied after transaction: {remaining:?}");
    }
    record(&dir, "verification.txt", "verified\n")?;
    println!("Requested dependencies verified. No Atelier binaries, rice settings or sessions were changed by the manager.");
    Ok(())
}


// Completion records are individually atomic and durable. A missing verification
// marker means the final dependency check is not established, even if pacman exited 0.
fn record(dir: &Path, name: &str, value: &str) -> Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(value.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(dir.join(name))?;
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}
fn checkpoint_file(path: &Path) -> Result<Option<fs::File>> {
    let file = match fs::OpenOptions::new().read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.permissions().mode() & 0o022 != 0 {
        bail!("unsafe checkpoint file: {}", path.display());
    }
    Ok(Some(file))
}
fn checkpoint_text(path: &Path) -> Result<Option<String>> {
    let Some(file) = checkpoint_file(path)? else { return Ok(None); };
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 { bail!("checkpoint metadata exceeds 64 KiB"); }
    Ok(Some(String::from_utf8(bytes)?))
}
#[derive(Debug, Serialize)]
pub(super) struct HistoryEntry {
    pub id: String,
    pub status: &'static str,
    pub targets: Vec<String>,
    pub before_inventory: bool,
    pub after_inventory: bool,
    outcome: Option<String>,
    pub error: Option<String>,
}
fn read_transaction(root: &Path, id: &str) -> Result<HistoryEntry> {
    let dir = root.join(id);
    checkpoint_directory(&dir)?;
    let before = checkpoint_file(&dir.join("before.txt"))?.is_some();
    let after = checkpoint_file(&dir.join("after.txt"))?.is_some();
    let targets = checkpoint_text(&dir.join("targets.txt"))?;
    let outcome = checkpoint_text(&dir.join("outcome.txt"))?;
    let verification = checkpoint_text(&dir.join("verification.txt"))?;
    let status = if !before || targets.is_none() { "incomplete-checkpoint" }
        else if outcome.is_none() { "outcome-unknown" }
        else {
            match verification.as_deref().map(str::trim) {
                Some("verified") => "verified",
                Some("launch-failed") => "launch-failed",
                Some("transaction-failed") => "transaction-failed",
                Some("verification-unavailable") => "verification-unavailable",
                Some("dependencies-unsatisfied") => "dependencies-unsatisfied",
                None => "final-verification-unknown",
                Some(_) => bail!("unknown checkpoint verification marker"),
            }
        };
    Ok(HistoryEntry { id: id.into(), status,
        targets: targets.unwrap_or_default().lines().map(str::to_owned).collect(),
        before_inventory: before, after_inventory: after, outcome, error: None })
}
fn history_at(root: &Path) -> Result<Vec<HistoryEntry>> {
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error.into()),
        Ok(_) => checkpoint_directory(root)?,
    }
    let mut ids = BTreeSet::new();
    for entry in fs::read_dir(root)? {
        if let Ok(id) = entry?.file_name().into_string() {
            if id.strip_prefix("transaction-").is_some_and(|suffix| !suffix.is_empty()
                && suffix.len() <= 64 && suffix.bytes().all(|b| b.is_ascii_alphanumeric())) {
                ids.insert(id);
            }
        }
    }
    Ok(ids.into_iter().map(|id| match read_transaction(root, &id) {
        Ok(entry) => entry,
        Err(error) => HistoryEntry { id, status: "invalid-checkpoint", targets: vec![],
            before_inventory: false, after_inventory: false, outcome: None, error: Some(format!("{error:#}")) },
    }).collect())
}
pub(super) fn history() -> Result<Vec<HistoryEntry>> {
    history_at(&checkpoint_root()?)
}
pub fn show_history(json: bool) -> Result<()> {
    let entries = history()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({"schema":1,"transactions":entries}))?);
    } else {
        if entries.is_empty() { println!("No package transaction checkpoints."); }
        for entry in entries {
            println!("{} [{}] — {}", entry.id, entry.status, entry.targets.join(", "));
            println!("  Before inventory: {}; after inventory: {}", entry.before_inventory, entry.after_inventory);
            if let Some(error) = entry.error { println!("  Checkpoint error: {error}"); }
        }
        println!("Historical results only; missing markers can indicate interruption or legacy records. No package repair or rollback is performed.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn package_handoff_accepts_only_catalog_products_and_known_providers() {
        let args = ["shell","--provider","media","--provider","audio"].map(str::to_owned);
        let (products, providers) = parse_install_args(&args).unwrap();
        assert_eq!(products, vec!["shell"]);
        assert_eq!(providers, vec!["media","audio"]);
        for invalid in [["shell","--provider","unknown"],["shell","--provider","--needed"],["shell","--bad","audio"]] {
            assert!(parse_install_args(&invalid.map(str::to_owned)).is_err());
        }
    }
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
    fn history_distinguishes_interrupted_legacy_and_verified_checkpoints() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("history");
        assert!(history_at(&root).unwrap().is_empty());
        assert!(!root.exists());
        let dir = root.join("transaction-A");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("before.txt"), "gtk4 1\n").unwrap();
        fs::write(dir.join("targets.txt"), "niri\n").unwrap();
        assert_eq!(history_at(&root).unwrap()[0].status, "outcome-unknown");
        record(&dir, "outcome.txt", "exit status: 0\n").unwrap();
        assert_eq!(history_at(&root).unwrap()[0].status, "final-verification-unknown");
        record(&dir, "verification.txt", "verified\n").unwrap();
        assert_eq!(history_at(&root).unwrap()[0].status, "verified");
        assert!(!history_at(&root).unwrap()[0].after_inventory);
        assert!(record(&dir, "verification.txt", "transaction-failed\n").is_err());
        fs::remove_file(dir.join("targets.txt")).unwrap();
        std::os::unix::fs::symlink(dir.join("before.txt"), dir.join("targets.txt")).unwrap();
        assert_eq!(history_at(&root).unwrap()[0].status, "invalid-checkpoint");
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
