//! Explicit user-local deployment staging. Never modifies PATH or an active session.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, env, fs::{self, File, OpenOptions}, io::{self, Read, Write}, os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt}, path::{Path, PathBuf}};
use crate::catalog;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Artifact { pub name: String, pub bytes: u64, pub sha256: String }
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Manifest {
    pub schema: u32,
    pub manager_version: String,
    pub source: PathBuf,
    pub products: Vec<String>,
    pub artifacts: Vec<Artifact>,
}

pub struct StagePlan { source: PathBuf, root: PathBuf, products: Vec<String>, binaries: Vec<&'static str> }

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
        Ok(Self { source, root, products: ids.to_vec(), binaries: binaries.into_iter().collect() })
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
fn copy_artifact(source: &Path, destination: &Path, name: &str) -> Result<Artifact> {
    verify_source(source)?;
    let mut input = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(source)?;
    let metadata = input.metadata()?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 { bail!("source changed during staging"); }
    let mut output = OpenOptions::new().write(true).create_new(true).mode(0o700).open(destination)?;
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
    output.sync_all()?;
    Ok(Artifact { name: name.to_owned(), bytes, sha256: format!("{:x}", digest.finalize()) })
}
fn stage(plan: StagePlan) -> Result<PathBuf> {
    ensure_root(&plan.root)?;
    let pending = tempfile::Builder::new().prefix("deployment-").tempdir_in(&plan.root)?;
    let bin = pending.path().join("bin");
    fs::create_dir(&bin)?;
    let mut artifacts = Vec::new();
    for name in &plan.binaries {
        artifacts.push(copy_artifact(&plan.source.join(name), &bin.join(name), name)?);
    }
    File::open(&bin)?.sync_all()?;
    let manifest = Manifest { schema: 1, manager_version: env!("CARGO_PKG_VERSION").into(), source: plan.source, products: plan.products, artifacts };
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
// Cancellation returns None; successful staging returns its deployment ID.
// This operation never changes the selection.
fn review_and_stage(plan: StagePlan, input: &mut impl io::BufRead, out: &mut impl Write) -> Result<Option<String>> {
    writeln!(out, "Stage native Atelier binaries from {} into a new private deployment under {}", plan.source.display(), plan.root.display())?;
    for name in &plan.binaries { writeln!(out, "  {name}")?; }
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

pub fn run(source: &Path, ids: &[String]) -> Result<Option<String>> {
    if nix::unistd::Uid::effective().is_root() { bail!("run staging as your desktop user, not root or sudo"); }
    let plan = StagePlan::prepare(source, data_root()?, ids)?;
    review_and_stage(plan, &mut io::stdin().lock(), &mut io::stdout().lock())
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
    fn symlink_artifact_is_refused() {
        let source = tempfile::tempdir().unwrap();
        artifact(source.path(), "other");
        std::os::unix::fs::symlink(source.path().join("other"), source.path().join("atelier-shell")).unwrap();
        assert!(StagePlan::prepare(source.path(), source.path().join("deployments"), &["shell".into()]).is_err());
    }
}
