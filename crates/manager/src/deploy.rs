//! Explicit user-local deployment staging. Never modifies PATH or an active session.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, env, fs::{self, File, OpenOptions}, io::{self, Read, Write}, os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt}, path::{Path, PathBuf}};
use crate::catalog;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Artifact { pub name: String, pub bytes: u64, pub sha256: String }
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Manifest {
    pub schema: u32,
    pub manager_version: String,
    pub source: PathBuf,
    pub products: Vec<String>,
    pub artifacts: Vec<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_deployment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<ReleaseOrigin>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct ReleaseOrigin {
    pub version: String,
    pub revision: String,
    pub sequence: u64,
    pub key_id: String,
    // None denotes a complete signed bundle (legacy release-stage records).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub covered_artifacts: Option<Vec<String>>,
}

struct StageArtifact { name: String, source: PathBuf, reviewed: Option<Artifact> }
pub struct StagePlan { source: PathBuf, root: PathBuf, products: Vec<String>, binaries: Vec<StageArtifact>, base_deployment: Option<String>, release: Option<ReleaseOrigin> }

pub(super) fn data_root() -> Result<PathBuf> {
    let base = env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .or_else(|| env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute()).map(|p| p.join(".local/share")))
        .context("no absolute XDG data directory or home directory")?;
    Ok(base.join("atelier-manager/deployments"))
}
fn verify_source(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).with_context(|| format!("inspect {}", path.display()))?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 || metadata.len() == 0 || metadata.len() > 1024 * 1024 * 1024 {
        bail!("not a regular nonempty executable within size limits: {}", path.display());
    }
    Ok(())
}
impl StagePlan {
    fn prepare(source: &Path, root: PathBuf, ids: &[String]) -> Result<Self> {
        let source = source.canonicalize().context("resolve trusted build directory")?;
        if !source.is_dir() { bail!("source must be a directory"); }
        let products = catalog::resolve(ids)?;
        if products.iter().any(|product| !matches!(product.source, "atelier-de" | "aggregate")) {
            bail!("staging supports native Atelier build artifacts only; external packages and Mango Atelier use separate installation workflows");
        }
        let binaries: BTreeSet<_> = products.iter().flat_map(|product| product.binaries.iter().copied()).collect();
        for name in &binaries { verify_source(&source.join(name))?; }
        let binaries = binaries.into_iter().map(|name| StageArtifact {
            name: name.into(), source: source.join(name), reviewed: None,
        }).collect();
        Ok(Self { source, root, products: ids.to_vec(), binaries, base_deployment: None, release: None })
    }
}

pub(super) fn ensure_root(root: &Path) -> Result<()> {
    fs::create_dir_all(root)?;
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() || metadata.uid() != nix::unistd::Uid::effective().as_raw() || metadata.permissions().mode() & 0o022 != 0 {
        bail!("deployment root must be a user-owned non-symlink directory not writable by others");
    }
    Ok(())
}
pub(super) fn read_artifact(source: &Path, name: &str, mut output: impl Write) -> Result<Artifact> {
    verify_source(source)?;
    let mut input = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(source)?;
    let metadata = input.metadata()?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 { bail!("source changed during staging"); }
    let mut digest = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 { break; }
        bytes += count as u64;
        if bytes > 1024 * 1024 * 1024 { bail!("artifact exceeded size limit"); }
        output.write_all(&buffer[..count])?;
        digest.update(&buffer[..count]);
    }
    if bytes == 0 || bytes != metadata.len() { bail!("artifact changed while copying"); }
    Ok(Artifact { name: name.to_owned(), bytes, sha256: format!("{:x}", digest.finalize()) })
}
fn copy_artifact(source: &Path, destination: &Path, name: &str) -> Result<Artifact> {
    let mut output = OpenOptions::new().write(true).create_new(true).mode(0o700).open(destination)?;
    let artifact = read_artifact(source, name, &mut output)?;
    output.sync_all()?;
    Ok(artifact)
}
fn stage(plan: StagePlan) -> Result<PathBuf> {
    stage_with(plan, |_| Ok(()))
}

fn stage_with(plan: StagePlan, before_complete: impl FnOnce(&Path) -> Result<()>) -> Result<PathBuf> {
    ensure_root(&plan.root)?;
    let _lifecycle = crate::selection::selection_lock(&plan.root)?;
    if let Some(base) = &plan.base_deployment { crate::selection::require_current(&plan.root, base)?; }
    let pending = tempfile::Builder::new().prefix("deployment-").tempdir_in(&plan.root)?;
    let bin = pending.path().join("bin");
    fs::create_dir(&bin)?;
    let mut artifacts = Vec::new();
    for item in &plan.binaries {
        let artifact = copy_artifact(&item.source, &bin.join(&item.name), &item.name)?;
        if item.reviewed.as_ref().is_some_and(|expected| expected != &artifact) {
            bail!("artifact changed after review: {}", item.name);
        }
        artifacts.push(artifact);
    }
    File::open(&bin)?.sync_all()?;
    let manifest = Manifest { schema: 1, manager_version: env!("CARGO_PKG_VERSION").into(), source: plan.source, products: plan.products, artifacts, base_deployment: plan.base_deployment, release: plan.release };
    before_complete(pending.path())?;
    // The manifest is the completion marker; interrupted partial directories
    // without this marker are not considered valid deployments.
    let mut marker = tempfile::NamedTempFile::new_in(pending.path())?;
    serde_json::to_writer_pretty(&mut marker, &manifest)?;
    marker.write_all(b"\n")?; marker.as_file().sync_all()?;
    marker.persist_noclobber(pending.path().join("manifest.json"))?;
    File::open(pending.path())?.sync_all()?;
    File::open(&plan.root)?.sync_all()?;
    Ok(pending.keep())
}
/// Stage only authenticated artifact identities; no execution or selection change.
pub(super) fn stage_release(source: &Path, release: &crate::release::ReleaseManifest, key_id: &str,
    before_complete: impl FnOnce(&Path) -> Result<()>) -> Result<PathBuf> {
    stage_release_at(source, data_root()?, release, key_id, before_complete)
}
fn stage_release_at(source: &Path, root: PathBuf, release: &crate::release::ReleaseManifest, key_id: &str,
    before_complete: impl FnOnce(&Path) -> Result<()>) -> Result<PathBuf> {
    release.validate_host()?;
    let mut plan = StagePlan::prepare(source, root, &release.products)?;
    for item in &mut plan.binaries {
        let expected = release.artifacts.iter().find(|a| a.name == item.name)
            .context("missing signed artifact identity")?;
        item.reviewed = Some(Artifact { name: expected.name.clone(), bytes: expected.bytes, sha256: expected.sha256.clone() });
    }
    plan.release = Some(ReleaseOrigin { version: release.version.clone(), revision: release.revision.clone(),
        sequence: release.sequence.context("release sequence required")?, key_id: key_id.into(), covered_artifacts: None });
    stage_with(plan, before_complete)
}

// Cancellation returns None; successful staging returns its deployment ID.
// This operation never changes the selection.
fn review_and_stage(plan: StagePlan, input: &mut impl io::BufRead, out: &mut impl Write) -> Result<Option<String>> {
    writeln!(out, "Stage native Atelier binaries from {} into a new private deployment under {}", plan.source.display(), plan.root.display())?;
    for item in &plan.binaries { writeln!(out, "  {}", item.name)?; }
    writeln!(out, "No existing installation, PATH, session, package or configuration will be modified. The source must be a trusted local build.")?;
    write!(out, "Stage these files? [y/N] ")?; out.flush()?;
    let mut answer = String::new(); input.read_line(&mut answer)?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        writeln!(out, "Staging cancelled. No changes made.")?;
        return Ok(None);
    }
    let path = stage(plan)?;
    let id = path.file_name().and_then(|name| name.to_str()).context("invalid generated deployment ID")?.to_owned();
    writeln!(out, "Deployment staged: {}", path.display())?;
    writeln!(out, "Binaries: {}. No activation was performed; previous deployments remain untouched.", path.join("bin").display())?;
    Ok(Some(id))
}

/// Review a local source build without writing anything. The returned opaque
/// plan pins the reviewed hashes for a later separately approved staging.
#[cfg(feature = "gui")]
pub(super) fn review_local(source: &Path, ids: &[String]) -> Result<(StagePlan, String)> {
    if nix::unistd::Uid::effective().is_root() { bail!("run staging as your desktop user, not root"); }
    if !source.is_absolute() { bail!("use an absolute trusted build directory"); }
    let mut plan = StagePlan::prepare(source, data_root()?, ids)?;
    let mut detail = format!("Trusted local build: {}\nNew deployment under: {}\n", plan.source.display(), plan.root.display());
    for item in &mut plan.binaries {
        let identity = read_artifact(&item.source, &item.name, io::sink())?;
        detail.push_str(&format!("{}: {} bytes\nSHA-256 {}\n", identity.name, identity.bytes, identity.sha256));
        item.reviewed = Some(identity);
    }
    Ok((plan, detail))
}

#[cfg(feature = "gui")]
pub(super) fn stage_reviewed(plan: StagePlan) -> Result<PathBuf> {
    if nix::unistd::Uid::effective().is_root() { bail!("run staging as your desktop user, not root"); }
    stage(plan)
}

pub fn run(source: &Path, ids: &[String]) -> Result<Option<String>> {
    if nix::unistd::Uid::effective().is_root() { bail!("run staging as your desktop user, not root or sudo"); }
    let plan = StagePlan::prepare(source, data_root()?, ids)?;
    review_and_stage(plan, &mut io::stdin().lock(), &mut io::stdout().lock())
}


#[derive(Debug, Serialize)]
struct Change {
    name: String,
    action: &'static str,
    before: Artifact,
    after: Artifact,
}
#[derive(Debug, Serialize)]
struct UpdatePreview {
    schema: u32,
    base_deployment: String,
    source: PathBuf,
    changes: Vec<Change>,
}
fn update_plan(source: &Path, root: PathBuf, id: String, base: Manifest, ids: &[String]) -> Result<(StagePlan, UpdatePreview)> {
    let source = source.canonicalize().context("resolve trusted candidate build directory")?;
    if !source.is_dir() { bail!("source must be a directory"); }
    let selected = catalog::resolve(ids)?;
    if selected.iter().any(|p| !matches!(p.source, "atelier-de" | "aggregate")) {
        bail!("only native products can be updated");
    }
    let targets: BTreeSet<_> = selected.iter().flat_map(|p| p.binaries.iter().copied()).collect();
    if targets.iter().any(|name| !base.artifacts.iter().any(|a| a.name == *name)) {
        bail!("update only supports products already in the selected deployment; stage a new bundle to add products");
    }
    let mut binaries = Vec::new();
    let mut changes = Vec::new();
    for old in base.artifacts {
        let retained = root.join(&id).join("bin").join(&old.name);
        let (action, path, expected) = if targets.contains(old.name.as_str()) {
            let candidate = source.join(&old.name);
            let next = read_artifact(&candidate, &old.name, io::sink())?;
            if next == old { ("unchanged", retained, old.clone()) } else { ("replace", candidate, next) }
        } else {
            ("retain", retained, old.clone())
        };
        binaries.push(StageArtifact { name: old.name.clone(), source: path, reviewed: Some(expected.clone()) });
        changes.push(Change { name: old.name.clone(), action, before: old, after: expected });
    }
    let preview = UpdatePreview { schema: 1, base_deployment: id.clone(), source: source.clone(), changes };
    Ok((StagePlan { source, root, products: base.products, binaries, base_deployment: Some(id), release: None }, preview))
}


/// An authenticated component release may replace only artifacts already in the
/// current deployment. Retained identities are pinned to the verified base.
fn signed_update_plan(source: &Path, root: PathBuf, id: String, base: Manifest,
    release: &crate::release::ReleaseManifest, key: &str) -> Result<(StagePlan, UpdatePreview)> {
    release.validate_host()?;
    let (mut plan, preview) = update_plan(source, root, id, base, &release.products)?;
    let mut covered = Vec::new();
    for expected in &release.artifacts {
        let item = plan.binaries.iter().find(|item| item.name == expected.name)
            .context("signed artifact is not part of the base deployment")?;
        let signed = Artifact { name: expected.name.clone(), bytes: expected.bytes, sha256: expected.sha256.clone() };
        if item.reviewed.as_ref() != Some(&signed) {
            bail!("candidate artifact differs from signed identity: {}", expected.name);
        }
        covered.push(expected.name.clone());
    }
    plan.release = Some(ReleaseOrigin { version: release.version.clone(), revision: release.revision.clone(),
        sequence: release.sequence.context("release sequence required")?, key_id: key.into(), covered_artifacts: Some(covered) });
    Ok((plan, preview))
}

pub(super) fn review_signed_update(source: &Path, release: &crate::release::ReleaseManifest, key: &str) -> Result<Option<StagePlan>> {
    let root = data_root()?;
    let (id, base) = crate::selection::verified_current(&root)?;
    let (plan, preview) = signed_update_plan(source, root, id, base, release, key)?;
    println!("Authenticated update of {} using release {}", preview.base_deployment, release.version);
    for change in &preview.changes {
        println!("  {}: {} ({} -> {} bytes)", change.name, change.action, change.before.bytes, change.after.bytes);
    }
    if !preview.changes.iter().any(|c| c.action == "replace") {
        println!("No binary changes; nothing staged or accepted.");
        return Ok(None);
    }
    Ok(Some(plan))
}

pub(super) fn complete_signed_update(plan: StagePlan, before_complete: impl FnOnce(&Path) -> Result<()>) -> Result<PathBuf> {
    let base = plan.base_deployment.as_deref().context("missing reviewed base")?;
    crate::selection::require_current(&plan.root, base)?;
    stage_with(plan, before_complete)
}

pub fn update(source: &Path, ids: &[String], preview_only: bool) -> Result<()> {
    if !preview_only && nix::unistd::Uid::effective().is_root() { bail!("run updates as your desktop user, not root"); }
    let root = data_root()?;
    let (id, base) = crate::selection::verified_current(&root)?;
    let (plan, preview) = update_plan(source, root, id, base, ids)?;
    if preview_only {
        println!("{}", serde_json::to_string_pretty(&preview)?);
        return Ok(());
    }
    println!("Update preview for {} from {}", preview.base_deployment, preview.source.display());
    for change in &preview.changes {
        println!("  {}: {} ({} -> {} bytes)", change.name, change.action, change.before.bytes, change.after.bytes);
    }
    if !preview.changes.iter().any(|change| change.action == "replace") {
        println!("No candidate binary changes; nothing staged.");
        return Ok(());
    }
    println!("The new bundle preserves all other components. Mixed build compatibility is not guaranteed.");
    println!("No selection, running session, settings or packages will be modified.");
    print!("Stage this update? [y/N] "); io::stdout().flush()?;
    let mut answer = String::new(); io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        println!("Update cancelled."); return Ok(());
    }
    crate::selection::require_current(&plan.root, &preview.base_deployment)?;
    let path = stage(plan)?;
    println!("Update staged at {}. Review and activate explicitly; the current deployment remains selected.", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn artifact(source: &Path, name: &str) {
        fs::write(source.join(name), b"fixture not executed").unwrap();
        fs::set_permissions(source.join(name), fs::Permissions::from_mode(0o700)).unwrap();
    }
    #[test]
    fn stages_separately_and_records_hashes_without_overwriting() {
        let source = tempfile::tempdir().unwrap(); let root = tempfile::tempdir().unwrap();
        artifact(source.path(), "atelier-shell");
        let ids = vec!["shell".into()];
        let first = stage(StagePlan::prepare(source.path(), root.path().to_owned(), &ids).unwrap()).unwrap();
        let second = stage(StagePlan::prepare(source.path(), root.path().to_owned(), &ids).unwrap()).unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(first.join("bin/atelier-shell")).unwrap(), b"fixture not executed");
        let manifest: serde_json::Value = serde_json::from_reader(File::open(second.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["artifacts"][0]["sha256"].as_str().unwrap().len(), 64);
    }
    #[test]
    fn missing_or_external_selection_is_rejected_before_writes() {
        let source = tempfile::tempdir().unwrap();
        let root = source.path().join("not-created");
        assert!(StagePlan::prepare(source.path(), root.clone(), &["session".into()]).is_err());
        assert!(StagePlan::prepare(source.path(), root.clone(), &["niri".into()]).is_err());
        assert!(!root.exists());
    }
    #[test]
    fn review_cancellation_leaves_root_untouched_and_acceptance_returns_id() {
        let source = tempfile::tempdir().unwrap();
        artifact(source.path(), "atelier-shell");
        let root = source.path().join("deployments");
        let ids = vec!["shell".into()];
        for answer in ["", "n\n", "q\n"] {
            let plan = StagePlan::prepare(source.path(), root.clone(), &ids).unwrap();
            assert!(review_and_stage(plan, &mut io::Cursor::new(answer), &mut Vec::new()).unwrap().is_none());
            assert!(!root.exists());
        }
        let plan = StagePlan::prepare(source.path(), root.clone(), &ids).unwrap();
        let id = review_and_stage(plan, &mut io::Cursor::new("yes\n"), &mut Vec::new()).unwrap().unwrap();
        assert!(root.join(id).join("manifest.json").is_file());
        assert!(!root.join("selection.json").exists());
    }
    #[test]
    fn selective_update_preserves_other_artifacts_and_rejects_changed_candidates() {
        let original = tempfile::tempdir().unwrap();
        let candidate = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        for name in ["atelier-shell", "atelier-settings"] { artifact(original.path(), name); }
        let ids = vec!["shell".into(), "settings".into()];
        let old_path = stage(StagePlan::prepare(original.path(), root.path().to_owned(), &ids).unwrap()).unwrap();
        let id = old_path.file_name().unwrap().to_str().unwrap().to_owned();
        crate::selection::select_for_test(root.path(), &id);
        let base = || serde_json::from_reader::<_, Manifest>(File::open(old_path.join("manifest.json")).unwrap()).unwrap();
        artifact(candidate.path(), "atelier-shell");
        let (_, unchanged) = update_plan(candidate.path(), root.path().to_owned(), id.clone(), base(), &["shell".into()]).unwrap();
        assert!(unchanged.changes.iter().all(|c| c.action != "replace"));
        fs::write(candidate.path().join("atelier-shell"), b"new shell").unwrap();
        let (plan, preview) = update_plan(candidate.path(), root.path().to_owned(), id.clone(), base(), &["shell".into()]).unwrap();
        assert_eq!(preview.changes.iter().filter(|c| c.action == "replace").count(), 1);
        let updated = stage(plan).unwrap();
        assert_eq!(fs::read(updated.join("bin/atelier-settings")).unwrap(), b"fixture not executed");
        assert_eq!(fs::read(updated.join("bin/atelier-shell")).unwrap(), b"new shell");
        assert_eq!(fs::read(old_path.join("bin/atelier-shell")).unwrap(), b"fixture not executed");
        let (plan, _) = update_plan(candidate.path(), root.path().to_owned(), id.clone(), base(), &["shell".into()]).unwrap();
        fs::write(candidate.path().join("atelier-shell"), b"changed after review").unwrap();
        assert!(stage(plan).is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("deployment-")).count(), 2);
        assert!(update_plan(candidate.path(), root.path().to_owned(), id, base(), &["session".into()]).is_err());
    }
    #[test]
    fn signed_component_update_preserves_base_and_marks_only_covered_artifacts() {
        let original = tempfile::tempdir().unwrap();
        let candidate = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        for name in ["atelier-shell", "atelier-settings"] { artifact(original.path(), name); }
        let base_path = stage(StagePlan::prepare(original.path(), root.path().to_owned(), &["shell".into(),"settings".into()]).unwrap()).unwrap();
        let id = base_path.file_name().unwrap().to_str().unwrap().to_owned();
        crate::selection::select_for_test(root.path(), &id);
        let base = || serde_json::from_reader::<_, Manifest>(File::open(base_path.join("manifest.json")).unwrap()).unwrap();
        artifact(candidate.path(), "atelier-shell");
        fs::write(candidate.path().join("atelier-shell"), b"signed new shell").unwrap();
        let release: crate::release::ReleaseManifest = serde_json::from_value(serde_json::json!({
            "schema":2,"sequence":8,"version":"v2","revision":"0123456789abcdef0123456789abcdef01234567",
            "os":"linux","arch":env::consts::ARCH,"products":["shell"],
            "artifacts":[{"name":"atelier-shell","bytes":b"signed new shell".len(),"sha256":format!("{:x}",Sha256::digest(b"signed new shell"))}]
        })).unwrap();
        let (plan, preview) = signed_update_plan(candidate.path(), root.path().to_owned(), id.clone(), base(), &release, "key").unwrap();
        assert_eq!(preview.changes.iter().filter(|c| c.action == "replace").count(),1);
        let path = stage(plan).unwrap();
        let manifest: Manifest = serde_json::from_reader(File::open(path.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest.base_deployment.as_deref(),Some(id.as_str()));
        assert_eq!(manifest.release.unwrap().covered_artifacts.unwrap(),vec!["atelier-shell"]);
        assert_eq!(fs::read(path.join("bin/atelier-settings")).unwrap(),b"fixture not executed");
        assert_eq!(fs::read(path.join("bin/atelier-shell")).unwrap(),b"signed new shell");
        fs::write(candidate.path().join("atelier-shell"),b"not signed").unwrap();
        assert!(signed_update_plan(candidate.path(), root.path().to_owned(), id, base(), &release, "key").is_err());
    }
    #[test]
    fn signed_staging_copies_only_reviewed_bytes_and_requires_final_check() {
        let source = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        artifact(source.path(), "atelier-shell");
        let hash = format!("{:x}", Sha256::digest(b"fixture not executed"));
        let release: crate::release::ReleaseManifest = serde_json::from_value(serde_json::json!({
            "schema":2,"sequence":7,"version":"v1","revision":"0123456789abcdef0123456789abcdef01234567",
            "os":"linux","arch":env::consts::ARCH,"products":["shell"],
            "artifacts":[{"name":"atelier-shell","bytes":b"fixture not executed".len(),"sha256":hash}]
        })).unwrap();
        assert!(stage_release_at(source.path(), root.path().to_owned(), &release, "key", |_| bail!("policy failure")).is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("deployment-")).count(), 0);
        let path = stage_release_at(source.path(), root.path().to_owned(), &release, "key", |_| Ok(())).unwrap();
        let manifest: Manifest = serde_json::from_reader(File::open(path.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest.release.unwrap().sequence, 7);
        fs::write(source.path().join("atelier-shell"), b"corrupt").unwrap();
        assert!(stage_release_at(source.path(), root.path().to_owned(), &release, "key", |_| Ok(())).is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("deployment-")).count(), 1);
    }
    #[test]
    fn symlink_artifact_is_refused() {
        let source = tempfile::tempdir().unwrap();
        artifact(source.path(), "other");
        std::os::unix::fs::symlink(source.path().join("other"), source.path().join("atelier-shell")).unwrap();
        assert!(StagePlan::prepare(source.path(), source.path().join("deployments"), &["shell".into()]).is_err());
    }
}
