//! Atomic selection and rollback of completed user-local deployments.
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs::{self, File, OpenOptions}, io::{self, IsTerminal, Read, Write}, os::{fd::AsRawFd, unix::{fs::{MetadataExt, OpenOptionsExt, PermissionsExt}, process::CommandExt}}, path::{Path, PathBuf}, process::Command};
use crate::{catalog, deploy::{data_root, ensure_root, Manifest}};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection { schema: u32, current: Option<String>, previous: Option<String> }
impl Default for Selection {
    fn default() -> Self { Self { schema: 1, current: None, previous: None } }
}
fn valid_id(id: &str) -> bool {
    id.strip_prefix("deployment-").is_some_and(|suffix| !suffix.is_empty() && suffix.len() <= 64 && suffix.bytes().all(|b| b.is_ascii_alphanumeric()))
}
fn owned(metadata: &fs::Metadata) -> bool {
    metadata.uid() == nix::unistd::Uid::effective().as_raw() && metadata.permissions().mode() & 0o022 == 0
}
fn directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || !owned(&metadata) { bail!("not an owned safe directory: {}", path.display()); }
    Ok(())
}
fn open_regular(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || !owned(&metadata) { return Err(io::Error::new(io::ErrorKind::InvalidData, "not an owned regular file")); }
    Ok(file)
}
fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let file = match open_regular(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    let mut bytes = Vec::new(); file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 { bail!("deployment metadata exceeds size limit"); }
    Ok(Some(serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?))
}
fn load_selection(root: &Path) -> Result<Selection> {
    let state: Selection = read_json(&root.join("selection.json"))?.unwrap_or_default();
    if state.schema != 1 || state.current.iter().chain(state.previous.iter()).any(|id| !valid_id(id))
        || (state.previous.is_some() && (state.current.is_none() || state.current == state.previous)) {
        bail!("invalid deployment selection");
    }
    Ok(state)
}
fn manifest(root: &Path, id: &str, hashes: bool) -> Result<Manifest> {
    if !valid_id(id) { bail!("invalid deployment ID"); }
    directory(root)?;
    let path = root.join(id); directory(&path)?;
    let bin = path.join("bin"); directory(&bin)?;
    let manifest: Manifest = read_json(&path.join("manifest.json"))?.context("deployment has no completion manifest")?;
    if manifest.schema != 1 { bail!("unsupported deployment manifest schema"); }
    if manifest.base_deployment.as_deref().is_some_and(|base| !valid_id(base) || base == id) { bail!("invalid deployment base identity"); }
    let products = catalog::resolve(&manifest.products)?;
    if products.iter().any(|product| !matches!(product.source, "atelier-de" | "aggregate")) { bail!("non-native deployment product"); }
    let expected: BTreeSet<_> = products.iter().flat_map(|product| product.binaries.iter().copied()).collect();
    let entries: BTreeSet<_> = manifest.artifacts.iter().map(|artifact| artifact.name.as_str()).collect();
    if entries != expected || entries.len() != manifest.artifacts.len() { bail!("deployment artifact list does not match selected products"); }
    let actual = fs::read_dir(&bin)?.map(|entry| {
        entry?.file_name().into_string().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid artifact filename"))
    }).collect::<io::Result<BTreeSet<_>>>()?;
    if actual.iter().map(String::as_str).collect::<BTreeSet<_>>() != expected { bail!("unexpected or missing artifact in deployment bin"); }
    for artifact in &manifest.artifacts {
        if artifact.bytes == 0 || artifact.bytes > 1024 * 1024 * 1024
            || artifact.sha256.len() != 64 || !artifact.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("invalid artifact metadata");
        }
        let mut file = open_regular(&bin.join(&artifact.name))?;
        let metadata = file.metadata()?;
        if metadata.len() != artifact.bytes || metadata.permissions().mode() & 0o111 == 0 { bail!("artifact size or mode mismatch: {}", artifact.name); }
        if hashes {
            let mut digest = Sha256::new();
            let mut bytes = 0u64; let mut buffer = [0u8; 64 * 1024];
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 { break; }
                bytes += count as u64;
                if bytes > artifact.bytes { bail!("artifact grew during verification"); }
                digest.update(&buffer[..count]);
            }
            if bytes != artifact.bytes || format!("{:x}", digest.finalize()) != artifact.sha256 { bail!("artifact hash mismatch: {}", artifact.name); }
        }
    }
    Ok(manifest)
}
fn write_selection(root: &Path, state: &Selection) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    serde_json::to_writer_pretty(&mut file, state)?;
    file.write_all(b"\n")?; file.as_file().sync_all()?;
    file.persist(root.join("selection.json"))?;
    File::open(root)?.sync_all()?;
    Ok(())
}
pub(super) fn selection_lock(root: &Path) -> Result<File> {
    directory(root)?;
    let lock = OpenOptions::new().read(true).write(true).create(true).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(root.join("selection.lock"))?;
    let metadata = lock.metadata()?;
    if !metadata.is_file() || !owned(&metadata) || metadata.nlink() != 1 { bail!("invalid deployment selection lock"); }
    FileExt::lock_exclusive(&lock)?;
    Ok(lock)
}
/// An ordinary activation of an update must still be based on the selected
/// bundle. Explicit historical selection and rollback intentionally bypass
/// this ancestry check, but never bypass artifact verification.
fn check_activation(state: &Selection, target: &str, manifest: &Manifest, historical: bool) -> Result<()> {
    if historical || state.current.as_deref() == Some(target) { return Ok(()); }
    if let Some(base) = &manifest.base_deployment {
        if state.current.as_deref() != Some(base.as_str()) {
            bail!("update was staged from {base}, not the current deployment; restage against the current bundle, or explicitly select it with activate ID --historical");
        }
    }
    Ok(())
}

#[cfg(test)]
fn switch(root: &Path, reviewed: &Selection, target: &str, historical: bool) -> Result<()> {
    switch_reviewed(root, reviewed, target, historical, None)
}
fn switch_reviewed(root: &Path, reviewed: &Selection, target: &str, historical: bool, identity: Option<&[u8]>) -> Result<()> {
    ensure_root(root)?;
    let _lock = selection_lock(root)?;
    let current = load_selection(root)?;
    if &current != reviewed { bail!("selection changed during review; retry"); }
    let target_manifest = manifest(root, target, true)?;
    if let Some(expected) = identity {
        if serde_json::to_vec(&target_manifest)?.as_slice() != expected { bail!("target manifest changed during review; retry"); }
    }
    check_activation(&current, target, &target_manifest, historical)?;
    if current.current.as_deref() == Some(target) { return Ok(()); }
    write_selection(root, &Selection { schema: 1, current: Some(target.to_owned()), previous: current.current })
}
#[derive(Debug, Serialize)]
pub(super) struct Inventory {
    schema: u32,
    pub current: Option<String>,
    pub previous: Option<String>,
    pub deployments: Vec<InventoryEntry>,
}
#[derive(Debug, Serialize)]
pub(super) struct InventoryEntry {
    pub id: String,
    pub role: &'static str,
    pub status: &'static str,
    pub manifest: Option<Manifest>,
    pub resolved_products: Vec<&'static str>,
    pub error: Option<String>,
}

fn inventory(root: &Path) -> Result<Inventory> {
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Inventory {
            schema: 1, current: None, previous: None, deployments: vec![],
        }),
        Err(error) => return Err(error.into()),
        Ok(_) => directory(root)?,
    }
    let selection = load_selection(root)?;
    let mut ids = BTreeSet::new();
    for entry in fs::read_dir(root)? {
        if let Ok(id) = entry?.file_name().into_string() {
            if valid_id(&id) { ids.insert(id); }
        }
    }
    // Include selected IDs even if their directories have been removed.
    ids.extend(selection.current.iter().chain(selection.previous.iter()).cloned());
    let mut deployments = Vec::new();
    for id in ids {
        let role = if selection.current.as_deref() == Some(&id) { "current" }
            else if selection.previous.as_deref() == Some(&id) { "previous" } else { "staged" };
        let entry = match manifest(root, &id, false) {
            Ok(manifest) => {
                let resolved_products = catalog::resolve(&manifest.products)?.iter().map(|product| product.id).collect();
                InventoryEntry { id, role, status: "structural-check-passed", manifest: Some(manifest), resolved_products, error: None }
            }
            Err(error) => InventoryEntry { id, role, status: "invalid-or-incomplete", manifest: None, resolved_products: vec![], error: Some(format!("{error:#}")) },
        };
        deployments.push(entry);
    }
    Ok(Inventory { schema: 1, current: selection.current, previous: selection.previous, deployments })
}

/// Full hash checks of current and previous selections, independently of each other.
pub(super) fn recovery() -> Result<serde_json::Value> {
    let root = data_root()?;
    recovery_at(&root)
}
fn recovery_at(root: &Path) -> Result<serde_json::Value> {
    let report = inventory(root)?;
    let check = |id: &Option<String>| match id {
        None => serde_json::json!({"id":null,"status":"not-selected"}),
        Some(id) => match manifest(root, id, true) {
            Ok(_) => serde_json::json!({"id":id,"status":"hashes-verified"}),
            Err(error) => serde_json::json!({"id":id,"status":"invalid","error":format!("{error:#}")}),
        },
    };
    Ok(serde_json::json!({
        "current":check(&report.current),
        "previous":check(&report.previous),
        "invalid_or_incomplete":report.deployments.iter().filter(|entry| entry.manifest.is_none())
            .map(|entry| &entry.id).collect::<Vec<_>>()
    }))
}


// A shared flock on a stable per-bundle inode survives exec and can be
// inherited by children. Never unlink or replace a runtime lease file.
// Absence of a held lock is not proof that unmanaged programs are stopped.
fn lease_file(bundle: &Path, create: bool) -> Result<File> {
    directory(bundle)?;
    let file = OpenOptions::new().read(true).write(true).create(create).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(bundle.join("runtime.lock"))?;
    let m = file.metadata()?;
    if !m.is_file() || !owned(&m) || m.nlink() != 1 { bail!("unsafe deployment runtime lock"); }
    Ok(file)
}
fn runtime_status(bundle: &Path) -> &'static str {
    let file = match lease_file(bundle, false) {
        Ok(file) => file,
        Err(error) if error.downcast_ref::<io::Error>().is_some_and(|e| e.kind() == io::ErrorKind::NotFound) => return "untracked",
        Err(_) => return "unknown",
    };
    match FileExt::try_lock_exclusive(&file) {
        Ok(()) => "no-managed-lease",
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => "managed-in-use",
        Err(_) => "unknown",
    }
}

pub(super) struct LeasedBin { bin: PathBuf, lease: File }
impl LeasedBin {
    pub(super) fn path(&self) -> &Path { &self.bin }
    pub(super) fn exec(self, mut command: Command) -> Result<()> {
        // Clear CLOEXEC only in the execing process. The lock remains held
        // across the supervisor's lifetime and inherited child descriptors.
        inherit_lease(&self.lease)?;
        Err(command.exec()).context("launch leased Atelier component")
    }
}
fn inherit_lease(file: &File) -> Result<()> {
    let fd = file.as_raw_fd();
    // SAFETY: fd belongs to the live File; fcntl only updates descriptor flags.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags == -1 { return Err(io::Error::last_os_error().into()); }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(())
}
fn lease_selected(root: &Path, required: &[&str]) -> Result<LeasedBin> {
    // Serialize selection and lease acquisition; future removal must take
    // this lock before testing the per-bundle exclusive lease.
    let _selection = selection_lock(root)?;
    let id = load_selection(root)?.current.context("no selected deployment")?;
    let m = manifest(root, &id, true)?;
    for name in required {
        if !m.artifacts.iter().any(|a| a.name == *name) {
            bail!("selected deployment lacks {name}");
        }
    }
    let bundle = root.join(id);
    let lease = lease_file(&bundle, true)?;
    FileExt::lock_shared(&lease)?;
    Ok(LeasedBin { bin: bundle.join("bin"), lease })
}
pub(super) fn leased_session(no_shell: bool) -> Result<LeasedBin> {
    let names = ["atelier-session", "atelier-compositor", "atelier-shell"];
    lease_selected(&data_root()?, &names[..if no_shell { 2 } else { 3 }])
}

fn launchable_product(id: &str) -> Result<&'static str> {
    match id {
        "shell" | "settings" | "services" | "notifications" => {
            let p = catalog::PRODUCTS.iter().find(|p| p.id == id).context("missing native catalog entry")?;
            Ok(p.binaries[0])
        }
        _ => bail!("run-product supports shell, settings, services and notifications; use session-launch for the nested desktop"),
    }
}
pub fn run_product(id: &str, args: &[String]) -> Result<()> {
    if nix::unistd::Uid::effective().is_root() { bail!("run as your desktop user, not root"); }
    let name = launchable_product(id)?;
    let leased = lease_selected(&data_root()?, &[name])?;
    let mut command = Command::new(leased.path().join(name));
    command.args(args);
    leased.exec(command)
}

fn cleanup_report(root: &Path) -> Result<serde_json::Value> {
    let report = inventory(root)?;
    let referenced: BTreeSet<_> = report.deployments.iter().filter_map(|entry| {
        entry.manifest.as_ref()?.base_deployment.as_deref()
    }).collect();
    let uncertain = report.deployments.iter().any(|entry| entry.manifest.is_none());
    let entries: Vec<_> = report.deployments.iter().map(|entry| {
        let runtime = runtime_status(&root.join(&entry.id));
        let reason = if entry.role == "current" || entry.role == "previous" { "selected-or-rollback" }
            else if referenced.contains(entry.id.as_str()) { "referenced-base" }
            else if runtime == "managed-in-use" { "managed-runtime-lease" }
            else if uncertain { "inventory-incomplete" }
            else if runtime == "unknown" { "runtime-status-unknown" }
            else { "offline-confirmation-required" };
        let bytes = entry.manifest.as_ref().map(|m| m.artifacts.iter().map(|a| a.bytes).sum::<u64>());
        serde_json::json!({"id":entry.id,"role":entry.role,"bytes":bytes,"protection":reason,"runtime":runtime,"removable":false,"offline_candidate":reason == "offline-confirmation-required"})
    }).collect();
    Ok(serde_json::json!({"schema":1,"deployments":entries,
        "note":"Read-only planning. Offline candidates require explicit confirmation and fresh validation; unmanaged runtime use is not detectable."}))
}

/// Under the lifecycle lock, validate a complete, unreferenced bundle. Incomplete
/// inventory fails closed because a missing manifest might conceal lineage.
fn removal_candidate(root: &Path, id: &str) -> Result<Manifest> {
    if !valid_id(id) { bail!("invalid deployment ID"); }
    let report = inventory(root)?;
    if report.current.as_deref() == Some(id) || report.previous.as_deref() == Some(id) {
        bail!("cannot remove current or rollback deployment");
    }
    if report.deployments.iter().any(|entry| entry.manifest.is_none()) {
        bail!("deployment inventory is incomplete; no removal attempted");
    }
    if report.deployments.iter().any(|entry| entry.manifest.as_ref().and_then(|m| m.base_deployment.as_deref()) == Some(id)) {
        bail!("deployment is referenced as an update base");
    }
    manifest(root, id, true)
}

/// Only known flat files may be removed. Never recursively traverse a deployment
/// or delete unrecognized user data. A late error leaves a quarantine directory.
fn removal_files(bundle: &Path, m: &Manifest) -> Result<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = m.artifacts.iter().map(|a| PathBuf::from("bin").join(&a.name)).collect();
    let mut top = BTreeSet::from(["bin", "manifest.json", "runtime.lock"]);
    if m.release.is_some() { top.extend(["release-manifest.json", "release-manifest.sig"]); }
    for entry in fs::read_dir(bundle)? {
        let entry = entry?;
        let name = entry.file_name().into_string().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid deployment filename"))?;
        if !top.contains(name.as_str()) { bail!("unexpected deployment entry: {name}"); }
        if name != "bin" {
            open_regular(&entry.path())?;
            paths.push(PathBuf::from(name));
        }
    }
    if m.release.is_some() {
        for name in ["release-manifest.json", "release-manifest.sig"] {
            open_regular(&bundle.join(name)).context("missing release provenance")?;
        }
    }
    Ok(paths)
}

#[cfg(test)]
fn remove_at(root: &Path, id: &str) -> Result<()> { remove_reviewed(root, id, None) }
fn remove_reviewed(root: &Path, id: &str, reviewed: Option<(&File, &[u8])>) -> Result<()> {
    let _lifecycle = selection_lock(root)?;
    let m = removal_candidate(root, id)?;
    let bundle = root.join(id);
    if let Some((directory, identity)) = reviewed {
        check_removal_identity(&bundle, directory, &m, identity)?;
    }
    if std::env::current_exe()?.canonicalize()?.starts_with(bundle.canonicalize()?) {
        bail!("cannot remove the deployment containing the running manager");
    }
    let lease = lease_file(&bundle, true)?;
    FileExt::try_lock_exclusive(&lease).context("deployment has a managed runtime lease or lock is unavailable")?;
    let files = removal_files(&bundle, &m)?;
    // Keep the quarantine even on a late failure; never let TempDir recursively
    // delete files we have not explicitly reviewed.
    let quarantine = tempfile::Builder::new().prefix("removed-deployment-").tempdir_in(root)?;
    let moved = quarantine.path().join("bundle");
    fs::rename(&bundle, &moved)?;
    let quarantine = quarantine.keep();
    File::open(&quarantine)?.sync_all()?;
    File::open(root)?.sync_all()?;
    let result = (|| -> Result<()> {
        for file in files { fs::remove_file(moved.join(file))?; }
        fs::remove_dir(moved.join("bin"))?;
        fs::remove_dir(&moved)?;
        fs::remove_dir(&quarantine)?;
        File::open(root)?.sync_all()?;
        Ok(())
    })();
    result.with_context(|| format!("removal interrupted; inspect quarantine {}", quarantine.display()))
}

fn check_removal_identity(bundle: &Path, reviewed: &File, manifest: &Manifest, identity: &[u8]) -> Result<()> {
    directory(bundle)?;
    let actual = fs::symlink_metadata(bundle)?;
    let old = reviewed.metadata()?;
    if actual.dev() != old.dev() || actual.ino() != old.ino() || serde_json::to_vec(manifest)?.as_slice() != identity {
        bail!("deployment identity changed since review; review again");
    }
    Ok(())
}
pub(super) struct OfflineRemoval {
    root: PathBuf,
    id: String,
    directory: File,
    identity: Vec<u8>,
    detail: String,
    phrase: String,
}
impl OfflineRemoval {
    pub(super) fn detail(&self) -> &str { &self.detail }
    pub(super) fn phrase(&self) -> &str { &self.phrase }
    pub(super) fn apply(self) -> Result<String> {
        if nix::unistd::Uid::effective().is_root() { bail!("run as your desktop user, not root"); }
        remove_reviewed(&self.root, &self.id, Some((&self.directory, &self.identity)))?;
        Ok(format!("Deployment {} removed.", self.id))
    }
}
fn review_removal_at(root: PathBuf, id: &str) -> Result<OfflineRemoval> {
    let _lock = selection_lock(&root)?;
    let m = removal_candidate(&root, id)?;
    let bundle = root.join(id);
    if std::env::current_exe()?.canonicalize()?.starts_with(bundle.canonicalize()?) {
        bail!("cannot remove the deployment containing the running manager");
    }
    if !matches!(runtime_status(&bundle), "untracked" | "no-managed-lease") {
        bail!("deployment has an active managed lease or unknown runtime status");
    }
    removal_files(&bundle, &m)?;
    let directory = OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW).open(&bundle)?;
    let identity = serde_json::to_vec(&m)?;
    check_removal_identity(&bundle, &directory, &m, &identity)?;
    let bytes = m.artifacts.iter().map(|a| a.bytes).sum::<u64>();
    let detail = format!("Permanently remove deployment {id} ({})?\n{} logical artifact bytes.\nClose ALL Atelier sessions and applications using this bundle, including direct launches. Use a separate manager executable and do not run older managers concurrently.\nCurrent selection, rollback target, referenced bases, packages, configuration and release trust remain untouched.", m.products.join(", "), bytes);
    Ok(OfflineRemoval { root, id: id.into(), directory, identity, detail, phrase: format!("remove offline {id}") })
}
pub(super) fn review_remove_offline(id: &str) -> Result<OfflineRemoval> {
    if nix::unistd::Uid::effective().is_root() { bail!("run as your desktop user, not root"); }
    review_removal_at(data_root()?, id)
}
pub fn remove_offline(id: &str) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() { bail!("interactive offline removal requires a terminal"); }
    let review = review_remove_offline(id)?;
    println!("{}", review.detail());
    print!("Type '{}' to confirm the bundle is not in use: ", review.phrase()); io::stdout().flush()?;
    let mut answer = String::new(); io::stdin().read_line(&mut answer)?;
    if answer.trim() != review.phrase() { println!("Cancelled."); return Ok(()); }
    println!("{}", review.apply()?);
    Ok(())
}

#[cfg(test)]
pub(super) fn select_for_test(root: &Path, id: &str) {
    switch(root, &load_selection(root).unwrap(), id, false).unwrap();
}

pub(super) fn cleanup_snapshot() -> Result<serde_json::Value> { cleanup_report(&data_root()?) }
pub fn cleanup_plan() -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&cleanup_snapshot()?)?);
    Ok(())
}

pub(super) fn snapshot() -> Result<Inventory> { inventory(&data_root()?) }

pub fn list(json: bool) -> Result<()> {
    let inventory = snapshot()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&inventory)?);
        return Ok(());
    }
    if inventory.deployments.is_empty() { println!("No deployments."); }
    for entry in inventory.deployments {
        match entry.manifest {
            Some(manifest) => println!("{} [{}] — {} (staged by manager {})", entry.id, entry.role, entry.resolved_products.join(", "), manifest.manager_version),
            None => println!("{} [{}] — incomplete or invalid: {}", entry.id, entry.role, entry.error.as_deref().unwrap_or("unknown error")),
        }
    }
    if let Some(id) = inventory.current { println!("Selected deployment: {id}"); } else { println!("No selected deployment."); }
    println!("Inventory checks structure and size only; use verify ID for full artifact hashing.");
    Ok(())
}

pub fn info(id: &str) -> Result<()> {
    let manifest = manifest(&data_root()?, id, true)?;
    let products: Vec<_> = catalog::resolve(&manifest.products)?.iter().map(|product| product.id).collect();
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
        "schema": 1, "id": id, "status": "hashes-verified",
        "resolved_products": products, "manifest": manifest
    }))?);
    Ok(())
}
pub(super) fn verified_current(root: &Path) -> Result<(String, Manifest)> {
    directory(root)?;
    let id = load_selection(root)?.current.context("no selected deployment")?;
    let verified = manifest(root, &id, true)?;
    Ok((id, verified))
}

pub(super) fn require_current(root: &Path, expected: &str) -> Result<()> {
    directory(root)?;
    if load_selection(root)?.current.as_deref() != Some(expected) {
        bail!("selected deployment changed during update review; retry");
    }
    Ok(())
}

pub(super) fn verify_identity(id: &str) -> Result<()> {
    manifest(&data_root()?, id, true)?;
    Ok(())
}
pub fn verify(id: &str) -> Result<()> {
    verify_identity(id)?;
    println!("Deployment {id}: all recorded artifacts verified.");
    Ok(())
}
/// Resolve the selected bundle and fully verify its recorded artifacts.
/// Required binaries must belong to this bundle, never a PATH fallback.
pub(super) fn verified_session_bin(no_shell: bool) -> Result<PathBuf> {
    let root = data_root()?;
    session_bin(&root, no_shell)
}

fn session_bin(root: &Path, no_shell: bool) -> Result<PathBuf> {
    directory(root)?;
    let id = load_selection(root)?.current.context("no selected deployment")?;
    let manifest = manifest(root, &id, true)?;
    let required = ["atelier-session", "atelier-compositor", "atelier-shell"];
    for name in &required[..if no_shell { 2 } else { 3 }] {
        if !manifest.artifacts.iter().any(|artifact| artifact.name == *name) {
            bail!("selected deployment lacks {name}; stage and select the required native products");
        }
    }
    Ok(root.join(id).join("bin"))
}

pub fn active_path() -> Result<()> {
    let root = data_root()?; directory(&root)?;
    let id = load_selection(&root)?.current.context("no selected deployment")?;
    manifest(&root, &id, false)?;
    println!("{}", root.join(id).join("bin").display()); Ok(())
}
/// An opaque, single-use review of the precise target and selection snapshot.
pub(super) struct ActivationReview {
    root: PathBuf,
    selected: Selection,
    target: String,
    historical: bool,
    identity: Vec<u8>,
    detail: String,
}
impl ActivationReview {
    pub(super) fn detail(&self) -> &str { &self.detail }
    pub(super) fn apply(self) -> Result<String> {
        if nix::unistd::Uid::effective().is_root() { bail!("run activation as your desktop user, not root"); }
        switch_reviewed(&self.root, &self.selected, &self.target, self.historical, Some(&self.identity))?;
        Ok(self.target)
    }
}

fn review_at(root: PathBuf, id: Option<&str>, historical: bool) -> Result<ActivationReview> {
    directory(&root)?;
    let selected = load_selection(&root)?;
    let target = match id { Some(id) => id.to_owned(), None => selected.previous.clone().context("no previous deployment to roll back to")? };
    let target_manifest = manifest(&root, &target, true)?;
    let historical = historical || id.is_none();
    check_activation(&selected, &target, &target_manifest, historical)?;
    let identity = serde_json::to_vec(&target_manifest)?;
    let mut detail = format!("Target: {target}\nCurrent: {}\nProducts: {}\nAll recorded artifact hashes verified.\nOnly the selection record changes; running sessions, PATH, settings and system packages are not modified.",
        selected.current.as_deref().unwrap_or("none"), target_manifest.products.join(", "));
    if historical { detail.push_str("\nHistorical selection may restore older components. This is local recovery, not remote release acceptance."); }
    Ok(ActivationReview { root, selected, target, historical, identity, detail })
}

pub(super) fn review_activation(id: Option<&str>, historical: bool) -> Result<ActivationReview> {
    if nix::unistd::Uid::effective().is_root() { bail!("run activation as your desktop user, not root"); }
    review_at(data_root()?, id, historical)
}

pub fn activate(id: Option<&str>, historical: bool) -> Result<()> {
    let review = review_activation(id, historical)?;
    println!("{}", review.detail());
    print!("Confirm selection? [y/N] "); io::stdout().flush()?;
    let mut reply = String::new(); io::stdin().read_line(&mut reply)?;
    if !matches!(reply.trim().to_ascii_lowercase().as_str(), "y" | "yes") { println!("Cancelled."); return Ok(()); }
    let target = review.apply()?;
    println!("Selected {target}. Use atelier-manager active-path for its bin directory.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path, id: &str, content: &[u8]) {
        let path = root.join(id); let bin = path.join("bin"); fs::create_dir_all(&bin).unwrap();
        let artifact = bin.join("atelier-shell"); fs::write(&artifact, content).unwrap();
        fs::set_permissions(&artifact, fs::Permissions::from_mode(0o700)).unwrap();
        let value = serde_json::json!({"schema":1,"manager_version":"test","source":"/trusted","products":["shell"],
            "artifacts":[{"name":"atelier-shell","bytes":content.len(),"sha256":format!("{:x}",Sha256::digest(content))}]});
        fs::write(path.join("manifest.json"), serde_json::to_vec(&value).unwrap()).unwrap();
    }
    #[test]
    fn selection_is_atomic_and_previous_can_be_restored() {
        let root = tempfile::tempdir().unwrap(); let root = root.path();
        fixture(root, "deployment-A", b"first"); fixture(root, "deployment-B", b"second");
        switch(root, &Selection::default(), "deployment-A", false).unwrap();
        let first = load_selection(root).unwrap();
        switch(root, &first, "deployment-B", false).unwrap();
        let second = load_selection(root).unwrap();
        assert_eq!(second.previous.as_deref(), Some("deployment-A"));
        assert!(switch(root, &first, "deployment-A", false).is_err());
        switch(root, &second, "deployment-A", false).unwrap();
        assert_eq!(load_selection(root).unwrap().current.as_deref(), Some("deployment-A"));
        assert!(root.join("deployment-B/bin/atelier-shell").exists());
    }
    #[test]
    fn managed_leases_survive_selection_changes_and_release_on_close() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fixture(root,"deployment-A",b"one");
        fixture(root,"deployment-B",b"two");
        switch(root,&Selection::default(), "deployment-A", false).unwrap();
        assert_eq!(runtime_status(&root.join("deployment-A")),"untracked");
        let first = lease_selected(root,&["atelier-shell"]).unwrap();
        let second = lease_selected(root,&["atelier-shell"]).unwrap();
        switch(root,&load_selection(root).unwrap(), "deployment-B", false).unwrap();
        assert_eq!(runtime_status(&root.join("deployment-A")),"managed-in-use");
        drop(first);
        assert_eq!(runtime_status(&root.join("deployment-A")),"managed-in-use");
        drop(second);
        assert_eq!(runtime_status(&root.join("deployment-A")),"no-managed-lease");
        assert!(lease_selected(root,&["atelier-settings"]).is_err());
    }
    #[test]
    fn lease_descriptor_survives_exec_until_child_exits() {
        let tmp = tempfile::tempdir().unwrap();
        let lease = lease_file(tmp.path(),true).unwrap();
        FileExt::lock_shared(&lease).unwrap();
        // Keep CLOEXEC in the multithreaded test runner. Otherwise concurrent
        // subprocesses (e.g. ssh-keygen) can inherit this unrelated lease.
        let fd = lease.as_raw_fd();
        let mut command = Command::new("/bin/sleep");
        command.arg("30");
        // SAFETY: only async-signal-safe fcntl calls in the forked child.
        unsafe {
            command.pre_exec(move || {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags == -1 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        drop(lease);
        let held = runtime_status(tmp.path());
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(held,"managed-in-use");
        assert_eq!(runtime_status(tmp.path()),"no-managed-lease");
    }
    #[test]
    fn product_launch_is_catalog_restricted() {
        assert_eq!(launchable_product("settings").unwrap(),"atelier-settings");
        assert!(launchable_product("desktop").is_err());
        assert!(launchable_product("session").is_err());
        assert!(launchable_product("../bin/sh").is_err());
    }
    #[test]
    fn activation_review_rejects_changed_manifest_even_if_new_hashes_are_valid() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fixture(root,"deployment-A",b"first");
        fixture(root,"deployment-B",b"second");
        switch(root,&Selection::default(),"deployment-A",false).unwrap();
        let review = review_at(root.to_owned(),Some("deployment-B"),false).unwrap();
        // A complete replacement with its own consistent manifest must still be rejected.
        fixture(root,"deployment-B",b"replacement");
        assert!(switch_reviewed(root,&review.selected,&review.target,review.historical,Some(&review.identity)).is_err());
        assert_eq!(load_selection(root).unwrap().current.as_deref(),Some("deployment-A"));
    }
    #[test]
    fn rollback_review_is_bound_to_the_exact_previous_selection() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for id in ["deployment-A","deployment-B","deployment-C"] { fixture(root,id,b"test"); }
        switch(root,&Selection::default(),"deployment-A",false).unwrap();
        switch(root,&load_selection(root).unwrap(),"deployment-B",false).unwrap();
        let review = review_at(root.to_owned(),None,false).unwrap();
        assert_eq!(review.target,"deployment-A");
        switch(root,&load_selection(root).unwrap(),"deployment-C",false).unwrap();
        assert!(switch_reviewed(root,&review.selected,&review.target,review.historical,Some(&review.identity)).is_err());
    }
    #[test]
    fn stale_update_requires_explicit_historical_selection() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for id in ["deployment-A", "deployment-B", "deployment-C"] { fixture(root,id,b"test"); }
        let path = root.join("deployment-C/manifest.json");
        let mut m: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        m["base_deployment"] = serde_json::json!("deployment-A");
        fs::write(&path,serde_json::to_vec(&m).unwrap()).unwrap();
        switch(root,&Selection::default(),"deployment-A",false).unwrap();
        switch(root,&load_selection(root).unwrap(),"deployment-C",false).unwrap();
        switch(root,&load_selection(root).unwrap(),"deployment-B",false).unwrap();
        let before = load_selection(root).unwrap();
        assert!(switch(root,&before,"deployment-C",false).is_err());
        assert_eq!(load_selection(root).unwrap(),before);
        switch(root,&before,"deployment-C",true).unwrap();
        assert_eq!(load_selection(root).unwrap().current.as_deref(),Some("deployment-C"));
        fs::write(root.join("deployment-C/bin/atelier-shell"),b"evil").unwrap();
        assert!(switch(root,&load_selection(root).unwrap(),"deployment-C",true).is_err());
    }
    #[test]
    fn reviewed_removal_refuses_replacement_and_new_selection() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fixture(root,"deployment-A",b"first");
        fixture(root,"deployment-B",b"second");
        let plan = review_removal_at(root.to_owned(),"deployment-B").unwrap();
        fixture(root,"deployment-B",b"changed bytes");
        assert!(remove_reviewed(root,"deployment-B",Some((&plan.directory,&plan.identity))).is_err());
        assert!(root.join("deployment-B").exists());
        let plan = review_removal_at(root.to_owned(),"deployment-B").unwrap();
        switch(root,&Selection::default(),"deployment-B",false).unwrap();
        assert!(remove_reviewed(root,"deployment-B",Some((&plan.directory,&plan.identity))).is_err());
        assert!(root.join("deployment-B").exists());
    }
    #[test]
    fn offline_removal_protects_selection_lineage_leases_and_unknown_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for id in ["deployment-A","deployment-B","deployment-C","deployment-D"] { fixture(root,id,b"test"); }
        switch(root,&Selection::default(), "deployment-A", false).unwrap();
        switch(root,&load_selection(root).unwrap(), "deployment-B", false).unwrap();
        assert!(remove_at(root,"deployment-A").is_err());
        assert!(remove_at(root,"deployment-B").is_err());
        let file = root.join("deployment-D/manifest.json");
        let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        value["base_deployment"] = serde_json::json!("deployment-C");
        fs::write(&file,serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(remove_at(root,"deployment-C").is_err());
        let lease = lease_file(&root.join("deployment-D"),true).unwrap();
        FileExt::lock_shared(&lease).unwrap();
        assert!(remove_at(root,"deployment-D").is_err());
        drop(lease);
        fs::write(root.join("deployment-D/keep"),b"user data").unwrap();
        assert!(remove_at(root,"deployment-D").is_err());
        fs::remove_file(root.join("deployment-D/keep")).unwrap();
        fs::create_dir(root.join("deployment-E")).unwrap();
        assert!(remove_at(root,"deployment-D").is_err());
        fs::remove_dir(root.join("deployment-E")).unwrap();
        remove_at(root,"deployment-D").unwrap();
        assert!(!root.join("deployment-D").exists());
        assert_eq!(load_selection(root).unwrap().current.as_deref(),Some("deployment-B"));
        remove_at(root,"deployment-C").unwrap();
        assert!(root.join("deployment-A").exists());
    }
    #[test]
    fn cleanup_report_protects_selection_and_referenced_bases() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        for id in ["deployment-A","deployment-B","deployment-C"] { fixture(root,id,b"test"); }
        let file = root.join("deployment-C/manifest.json");
        let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        value["base_deployment"] = serde_json::json!("deployment-A");
        fs::write(&file,serde_json::to_vec(&value).unwrap()).unwrap();
        switch(root,&Selection::default(), "deployment-B", false).unwrap();
        let report = cleanup_report(root).unwrap();
        assert_eq!(report["deployments"][0]["protection"],"referenced-base");
        assert_eq!(report["deployments"][1]["protection"],"selected-or-rollback");
        assert_eq!(report["deployments"][2]["removable"],false);
    }
    #[test]
    fn recovery_checks_current_and_previous_independently() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        fixture(root, "deployment-A", b"first");
        fixture(root, "deployment-B", b"second");
        switch(root, &Selection::default(), "deployment-A", false).unwrap();
        switch(root, &load_selection(root).unwrap(), "deployment-B", false).unwrap();
        fs::write(root.join("deployment-B/bin/atelier-shell"), b"broken").unwrap();
        let report = recovery_at(root).unwrap();
        assert_eq!(report["current"]["status"], "invalid");
        assert_eq!(report["previous"]["status"], "hashes-verified");
        assert_eq!(load_selection(root).unwrap().current.as_deref(), Some("deployment-B"));
    }
    #[test]
    fn inventory_tracks_selection_and_missing_deployments_without_mutating() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("deployments");
        assert!(inventory(&root).unwrap().deployments.is_empty());
        assert!(!root.exists());
        fs::create_dir(&root).unwrap();
        fixture(&root, "deployment-A", b"one");
        fixture(&root, "deployment-B", b"two");
        switch(&root, &Selection::default(), "deployment-A", false).unwrap();
        switch(&root, &load_selection(&root).unwrap(), "deployment-B", false).unwrap();
        fs::remove_dir_all(root.join("deployment-A")).unwrap();
        let report = inventory(&root).unwrap();
        assert_eq!(report.current.as_deref(), Some("deployment-B"));
        assert_eq!(report.previous.as_deref(), Some("deployment-A"));
        assert_eq!(report.deployments[0].role, "previous");
        assert_eq!(report.deployments[0].status, "invalid-or-incomplete");
        assert_eq!(report.deployments[1].status, "structural-check-passed");
        assert_eq!(report.deployments[1].resolved_products, vec!["shell"]);
        assert_eq!(report.deployments[1].manifest.as_ref().unwrap().manager_version, "test");
    }
    #[test]
    fn session_launch_requires_a_verified_complete_selected_bundle() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        assert!(session_bin(root, false).is_err());
        fixture(root, "deployment-A", b"shell only");
        switch(root, &Selection::default(), "deployment-A", false).unwrap();
        assert!(session_bin(root, false).is_err());
        assert!(session_bin(root, true).is_err());

        let id = "deployment-B";
        let dir = root.join(id);
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let content = b"fixture";
        let mut artifacts = Vec::new();
        for name in ["atelier-session", "atelier-compositor", "atelier-shell"] {
            fs::write(bin.join(name), content).unwrap();
            fs::set_permissions(bin.join(name), fs::Permissions::from_mode(0o700)).unwrap();
            artifacts.push(serde_json::json!({"name":name,"bytes":content.len(),"sha256":format!("{:x}",Sha256::digest(content))}));
        }
        fs::write(dir.join("manifest.json"), serde_json::to_vec(&serde_json::json!({
            "schema":1,"manager_version":"test","source":"/trusted",
            "products":["session","shell"],"artifacts":artifacts
        })).unwrap()).unwrap();
        let old = load_selection(root).unwrap();
        switch(root, &old, id, false).unwrap();
        assert_eq!(session_bin(root, false).unwrap(), bin);
        assert_eq!(session_bin(root, true).unwrap(), bin);
        fs::write(bin.join("atelier-compositor"), b"altered").unwrap();
        assert!(session_bin(root, false).is_err());
    }
    #[test]
    fn corrupt_or_untrusted_files_do_not_change_selection() {
        let root = tempfile::tempdir().unwrap(); let root = root.path();
        fixture(root, "deployment-A", b"first");
        fs::write(root.join("deployment-A/bin/atelier-shell"), b"other").unwrap();
        assert!(switch(root, &Selection::default(), "deployment-A", false).is_err());
        assert_eq!(load_selection(root).unwrap(), Selection::default());
        assert!(manifest(root, "../invalid", false).is_err());
    }
}
