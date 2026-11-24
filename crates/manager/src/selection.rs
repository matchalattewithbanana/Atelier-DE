//! Atomic selection and rollback of completed user-local deployments.
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs::{self, File, OpenOptions}, io::{self, Read, Write}, os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt}, path::{Path, PathBuf}};
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
fn switch(root: &Path, reviewed: &Selection, target: &str) -> Result<()> {
    ensure_root(root)?;
    let lock = OpenOptions::new().read(true).write(true).create(true).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(root.join("selection.lock"))?;
    let metadata = lock.metadata()?;
    if !metadata.is_file() || !owned(&metadata) { bail!("invalid deployment selection lock"); }
    FileExt::lock_exclusive(&lock)?;
    let current = load_selection(root)?;
    if &current != reviewed { bail!("selection changed during review; retry"); }
    manifest(root, target, true)?;
    if current.current.as_deref() == Some(target) { return Ok(()); }
    write_selection(root, &Selection { schema: 1, current: Some(target.to_owned()), previous: current.current })
}
#[derive(Debug, Serialize)]
struct Inventory {
    schema: u32,
    current: Option<String>,
    previous: Option<String>,
    deployments: Vec<InventoryEntry>,
}
#[derive(Debug, Serialize)]
struct InventoryEntry {
    id: String,
    role: &'static str,
    status: &'static str,
    manifest: Option<Manifest>,
    resolved_products: Vec<&'static str>,
    error: Option<String>,
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

pub fn list(json: bool) -> Result<()> {
    let inventory = inventory(&data_root()?)?;
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
pub fn verify(id: &str) -> Result<()> {
    manifest(&data_root()?, id, true)?;
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
pub fn activate(id: Option<&str>) -> Result<()> {
    if nix::unistd::Uid::effective().is_root() { bail!("run activation as your desktop user, not root"); }
    let root = data_root()?; directory(&root)?;
    let selected = load_selection(&root)?;
    let target = match id { Some(id) => id, None => selected.previous.as_deref().context("no previous deployment to roll back to")? };
    let manifest = manifest(&root, target, false)?;
    println!("Select deployment {target} ({})? Current: {}.", manifest.products.join(", "), selected.current.as_deref().unwrap_or("none"));
    println!("Only Atelier's selection record changes. No running processes, PATH, settings or system packages are modified.");
    print!("Confirm selection? [y/N] "); io::stdout().flush()?;
    let mut reply = String::new(); io::stdin().read_line(&mut reply)?;
    if !matches!(reply.trim().to_ascii_lowercase().as_str(), "y" | "yes") { println!("Cancelled."); return Ok(()); }
    switch(&root, &selected, target)?;
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
        switch(root, &Selection::default(), "deployment-A").unwrap();
        let first = load_selection(root).unwrap();
        switch(root, &first, "deployment-B").unwrap();
        let second = load_selection(root).unwrap();
        assert_eq!(second.previous.as_deref(), Some("deployment-A"));
        assert!(switch(root, &first, "deployment-A").is_err());
        switch(root, &second, "deployment-A").unwrap();
        assert_eq!(load_selection(root).unwrap().current.as_deref(), Some("deployment-A"));
        assert!(root.join("deployment-B/bin/atelier-shell").exists());
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
        switch(&root, &Selection::default(), "deployment-A").unwrap();
        switch(&root, &load_selection(&root).unwrap(), "deployment-B").unwrap();
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
        switch(root, &Selection::default(), "deployment-A").unwrap();
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
        switch(root, &old, id).unwrap();
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
        assert!(switch(root, &Selection::default(), "deployment-A").is_err());
        assert_eq!(load_selection(root).unwrap(), Selection::default());
        assert!(manifest(root, "../invalid", false).is_err());
    }
}
